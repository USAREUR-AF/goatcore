"""The edge role's key-storage comparison reads the core where its router reads.

Runs the role's tasks/key-storage-check.yml on localhost with a fake docker that
models one running core router: the arguments it was started with and the one
path inside it that holds its configuration. A core run by hand
(`docker run ... -c /etc/goat-core/config.json5`) keeps its configuration
somewhere other than the goat-zenoh role's /etc/zenoh/config.json5.
"""
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import tempfile
import unittest

import yaml

ROOT = Path(__file__).resolve().parents[1]
TASK = ROOT / 'ops/ansible/roles/goat-zenoh-edge/tasks/key-storage-check.yml'
FILTERS = ROOT / 'ops/ansible/filter_plugins'
GRANT = ['keys/weather/**']
HAND = (['-c', '/etc/goat-core/config.json5'], '/etc/goat-core/config.json5')
ROLE = (['--config', '/etc/zenoh/config.json5'], '/etc/zenoh/config.json5')


def config(*scopes):
    return json.dumps({'plugins': {'storage_manager': {'storages': {
        'store_' + str(i): {'key_expr': scope, 'volume': 'memory'} for i, scope in enumerate(scopes)
    }}}})


@unittest.skipUnless(shutil.which('ansible-playbook'), 'ansible-playbook is required')
class KeyStorageCheck(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        bindir = self.root / 'bin'
        bindir.mkdir()
        docker = bindir / 'docker'
        docker.write_text(
            '#!/bin/sh\n'
            '. "$KEYS_ROOT/core.env"\n'
            'printf "%s\\n" "$*" >> "$KEYS_ROOT/trace"\n'
            'case "$1" in\n'
            '  ps) printf "cid\\txaa-zenoh-router\\tzenoh-core\\n"; exit 0 ;;\n'
            '  inspect) printf "%s\\n" "$CORE_ARGS"; exit 0 ;;\n'
            '  exec) if [ "$4" = "$CORE_CONFIG_PATH" ]; then cat "$KEYS_ROOT/config.json5"; exit 0; fi\n'
            '    echo "cat: can\'t open \'$4\': No such file or directory" >&2; exit 1 ;;\n'
            'esac\n'
            'exit 1\n')
        docker.chmod(0o755)
        self.env = {**os.environ, 'PATH': str(bindir) + os.pathsep + os.environ['PATH'],
                    'KEYS_ROOT': str(self.root), 'ANSIBLE_FILTER_PLUGINS': str(FILTERS),
                    'ANSIBLE_LOCAL_TEMP': str(self.root / 'ansible-local'),
                    'ANSIBLE_REMOTE_TEMP': str(self.root / 'ansible-remote'),
                    'ANSIBLE_NOCOLOR': '1', 'ANSIBLE_STDOUT_CALLBACK': 'default',
                    'ANSIBLE_RETRY_FILES_ENABLED': '0',
                    'OBJC_DISABLE_INITIALIZE_FORK_SAFETY': 'YES'}

    def run_play(self, text, args, config_path, extra=None):
        (self.root / 'config.json5').write_text(text)
        (self.root / 'core.env').write_text(
            'CORE_ARGS=' + shlex.quote(json.dumps(args))
            + '\nCORE_CONFIG_PATH=' + shlex.quote(config_path) + '\n')
        play = [{'hosts': 'localhost', 'connection': 'local', 'gather_facts': False,
                 'become': False, 'vars': {
                     'ansible_python_interpreter': sys.executable,
                     'goat_zenoh_edge_key_bundle_key_exprs': GRANT, **(extra or {})},
                 'tasks': [{'ansible.builtin.include_tasks': str(TASK)}]}]
        path = self.root / 'play.yml'
        path.write_text(yaml.safe_dump(play, sort_keys=False))
        return subprocess.run(['ansible-playbook', '-i', 'localhost,', str(path)], env=self.env,
                              capture_output=True, text=True, timeout=90)

    def test_a_wide_key_storage_on_a_hand_run_core_is_refused(self):
        result = self.run_play(config('keys/**'), *HAND)
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn('which is WIDER', result.stdout)
        self.assertIn('exec xaa-zenoh-router cat /etc/goat-core/config.json5',
                      (self.root / 'trace').read_text())

    def test_a_scoped_key_storage_on_a_hand_run_core_is_compared(self):
        result = self.run_play(config('keys/weather/**'), *HAND)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertNotIn('"msg": "could not run', result.stdout)
        self.assertNotIn('WIDER', result.stdout)

    def test_a_wide_key_storage_on_a_role_core_is_refused(self):
        result = self.run_play(config('keys/**'), *ROLE)
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn('which is WIDER', result.stdout)

    def test_an_unreadable_configuration_is_reported_with_why_and_does_not_pass(self):
        result = self.run_play(config('keys/**'), ['-c', '/etc/missing.json5'], '/etc/goat-core/config.json5')
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn('could not run: the core key-storage comparison was not made', result.stdout)
        self.assertIn('could not read /etc/missing.json5 from xaa-zenoh-router', result.stdout)
        self.assertIn('cat exited 1', result.stdout)
        self.assertIn('No such file or directory', result.stdout)
        self.assertNotIn('storage_manager', result.stdout)

    def test_an_explicit_config_path_wins_over_the_arguments(self):
        result = self.run_play(config('keys/**'), ['-c', '/etc/elsewhere.json5'],
                               '/etc/goat-core/config.json5',
                               extra={'goat_zenoh_edge_core_config_path': '/etc/goat-core/config.json5'})
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn('which is WIDER', result.stdout)


if __name__ == '__main__':
    unittest.main()
