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


def require_ansible_playbook(case):
    """Skip without ansible-playbook, unless GOAT_TEST_REQUIRE_ANSIBLE=1 (CI sets
    it), where its absence fails the test instead of skipping it quietly."""
    if shutil.which('ansible-playbook'):
        return
    if os.environ.get('GOAT_TEST_REQUIRE_ANSIBLE') == '1':
        case.fail('GOAT_TEST_REQUIRE_ANSIBLE=1 but ansible-playbook is not on PATH')
    case.skipTest('ansible-playbook is required')


def config(*scopes):
    return json.dumps({'plugins': {'storage_manager': {'storages': {
        'store_' + str(i): {'key_expr': scope, 'volume': 'memory'} for i, scope in enumerate(scopes)
    }}}})


class KeyStorageCheck(unittest.TestCase):
    def setUp(self):
        require_ansible_playbook(self)
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        bindir = self.root / 'bin'
        bindir.mkdir()
        docker = bindir / 'docker'
        docker.write_text(
            '#!/bin/sh\n'
            '. "$KEYS_ROOT/core.env"\n'
            '[ "$1" = ps ] || printf "%s\\n" "$*" >> "$KEYS_ROOT/trace"\n'
            'case "$1" in\n'
            '  ps) printf "${KEYS_PS-cid\\txaa-zenoh-router\\tzenoh-core\\n}"; exit 0 ;;\n'
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

    def run_play(self, text, args, config_path, extra=None, ps=None):
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
        env = dict(self.env, **({'KEYS_PS': ps} if ps is not None else {}))
        return subprocess.run(['ansible-playbook', '-i', 'localhost,', str(path)], env=env,
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

    def test_an_unreadable_configuration_fails_the_play_with_why(self):
        result = self.run_play(config('keys/**'), ['-c', '/etc/missing.json5'], '/etc/goat-core/config.json5')
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn('could not read /etc/missing.json5 from xaa-zenoh-router on localhost', result.stdout)
        self.assertNotIn('PASS:', result.stdout)
        self.assertIn('cat exited 1', result.stdout)
        self.assertIn('No such file or directory', result.stdout)
        self.assertNotIn('storage_manager', result.stdout)

    def test_an_override_that_differs_from_the_arguments_is_refused(self):
        result = self.run_play(config('keys/weather/**'), ['-c', '/etc/elsewhere.json5'],
                               '/etc/goat-core/config.json5',
                               extra={'goat_zenoh_edge_core_config_path': '/etc/goat-core/config.json5'})
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn('/etc/elsewhere.json5', result.stdout)
        self.assertIn('goat_zenoh_edge_core_config_path is /etc/goat-core/config.json5', result.stdout)

    def test_an_override_names_the_path_for_a_router_whose_arguments_name_none(self):
        result = self.run_play(config('keys/**'), [], '/srv/core.json5',
                               extra={'goat_zenoh_edge_core_config_path': '/srv/core.json5'})
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn('which is WIDER', result.stdout)

    def test_a_scoped_store_reports_a_pass(self):
        result = self.run_play(config('keys/weather/**'), *HAND)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn('PASS: the core', result.stdout)

    def test_no_core_router_fails_the_play(self):
        result = self.run_play(config('keys/**'), *HAND, extra={}, ps='')
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn('could not run: the core key-storage comparison found no core to read', result.stdout)
        self.assertNotIn('PASS:', result.stdout)

    def test_the_opt_out_is_printed_as_a_decision_and_reads_nothing(self):
        result = self.run_play(config('keys/**'), *HAND, extra={'goat_zenoh_edge_key_scope_check': False})
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn('SKIP, by decision: goat_zenoh_edge_key_scope_check is false', result.stdout)
        self.assertIn('This is not a pass', result.stdout)
        self.assertFalse((self.root / 'trace').exists())


class KeyStorageOnAnotherHost(unittest.TestCase):
    """The deployed shape: the core router on its own host, in <partner>_core.

    Both inventory hosts connect locally; each one's Python interpreter names
    the host to a fake docker, so only the core host runs a core router.
    """

    def setUp(self):
        require_ansible_playbook(self)
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        bindir = self.root / 'bin'
        bindir.mkdir()
        docker = bindir / 'docker'
        docker.write_text(
            '#!/bin/sh\n'
            '[ "$1" = ps ] || printf "%s %s\\n" "$KEYS_HOST" "$*" >> "$KEYS_ROOT/trace"\n'
            '[ "$KEYS_HOST" = core ] || { [ "$1" = ps ] && exit 0; exit 1; }\n'
            'case "$1" in\n'
            '  ps) printf "cid\\txab-zenoh-router\\tzenoh-core\\n"; exit 0 ;;\n'
            '  inspect) printf \'["-c","/etc/goat-core/config.json5"]\\n\'; exit 0 ;;\n'
            '  exec) [ "$4" = /etc/goat-core/config.json5 ] && [ "$KEYS_UNREADABLE" != 1 ] && exec cat "$KEYS_ROOT/config.json5"\n'
            '    echo "cat: can\'t open \'$4\': Permission denied" >&2; exit 1 ;;\n'
            'esac\n'
            'exit 1\n')
        docker.chmod(0o755)
        for host in ('edge', 'core'):
            wrapper = self.root / ('python-' + host)
            wrapper.write_text('#!/bin/sh\nKEYS_HOST=' + host + ' PATH="' + str(bindir)
                               + ':$PATH" exec "' + sys.executable + '" "$@"\n')
            wrapper.chmod(0o755)
        self.env = {**os.environ, 'KEYS_ROOT': str(self.root), 'ANSIBLE_FILTER_PLUGINS': str(FILTERS),
                    'ANSIBLE_LOCAL_TEMP': str(self.root / 'ansible-local'),
                    'ANSIBLE_REMOTE_TEMP': str(self.root / 'ansible-remote'),
                    'ANSIBLE_NOCOLOR': '1', 'ANSIBLE_STDOUT_CALLBACK': 'default',
                    'ANSIBLE_RETRY_FILES_ENABLED': '0',
                    'OBJC_DISABLE_INITIALIZE_FORK_SAFETY': 'YES'}

    def run_play(self, text, cores=('core-1',), unreadable=False):
        (self.root / 'config.json5').write_text(text)

        def host(name, kind):
            return (name + ' ansible_connection=local ansible_python_interpreter='
                    + str(self.root / ('python-' + kind)))
        inventory = self.root / 'peering-edges.ini'
        inventory.write_text('\n'.join(['[xab_edge]', host('edge-1', 'edge'), '', '[xab_core]']
                                       + [host(c, 'core') for c in cores]
                                       + ['', '[peering_edges:children]', 'xab_edge', '',
                                          '[peering_cores:children]', 'xab_core', '']))
        play = [{'hosts': 'xab_edge', 'gather_facts': False, 'become': False, 'vars': {
                     'goat_zenoh_edge_partner_id': 'xab',
                     'goat_zenoh_edge_key_bundle_key_exprs': GRANT},
                 'tasks': [{'ansible.builtin.include_tasks': str(TASK)}]}]
        path = self.root / 'play.yml'
        path.write_text(yaml.safe_dump(play, sort_keys=False))
        return subprocess.run(['ansible-playbook', '-i', str(inventory), '--limit', 'xab_edge', str(path)],
                              env=dict(self.env, KEYS_UNREADABLE='1' if unreadable else '0'),
                              capture_output=True, text=True, timeout=90)

    def trace(self):
        path = self.root / 'trace'
        return path.read_text() if path.exists() else ''

    def test_a_wide_key_storage_on_a_separate_core_host_is_refused(self):
        result = self.run_play(config('keys/**'))
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn('which is WIDER', result.stdout)
        self.assertIn('core exec xab-zenoh-router cat /etc/goat-core/config.json5', self.trace())
        self.assertNotIn('edge exec', self.trace())

    def test_a_scoped_key_storage_on_a_separate_core_host_passes(self):
        result = self.run_play(config('keys/weather/**'))
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn('PASS: the core', result.stdout)

    def test_an_unreadable_configuration_on_the_core_host_fails_the_play(self):
        result = self.run_play(config('keys/**'), unreadable=True)
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn('could not read /etc/goat-core/config.json5 from xab-zenoh-router on core-1: '
                      'docker exec ... cat exited 1', result.stdout)
        self.assertIn('Permission denied', result.stdout)

    def test_no_core_host_fails_the_play(self):
        result = self.run_play(config('keys/**'), cores=())
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn('no host in inventory group xab_core', result.stdout)


if __name__ == '__main__':
    unittest.main()
