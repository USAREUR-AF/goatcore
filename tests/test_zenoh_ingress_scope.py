"""Scoped receive-storage preflight, including the retained legacy-store upgrade."""
import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest.mock

import yaml

ROOT = Path(__file__).resolve().parents[1]
PLUGIN = ROOT / 'ops/ansible/filter_plugins/zenoh_ingress.py'
SPEC = importlib.util.spec_from_file_location('zenoh_ingress_test', PLUGIN)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)
check = MODULE.zenoh_ingress_storage_check
REQUIRED = ['from/xaa/**']


def config(*scopes):
    return json.dumps({'plugins': {'storage_manager': {'storages': {
        'store_' + str(i): {'key_expr': scope} for i, scope in enumerate(scopes)
    }}}})


class ScopeInspection(unittest.TestCase):
    def test_legacy_plus_scoped_is_accepted(self):
        report = check(config('from/**', *REQUIRED), REQUIRED)
        self.assertEqual(report['missing'], [])
        self.assertEqual(report['error'], '')

    def test_scoped_only_is_accepted(self):
        self.assertEqual(check(config(*REQUIRED), REQUIRED)['missing'], [])

    def test_legacy_only_does_not_cover_a_scoped_declaration(self):
        self.assertEqual(check(config('from/**'), REQUIRED)['missing'], REQUIRED)

    def test_other_partner_does_not_cover_our_scope(self):
        self.assertEqual(check(config('from/xab/**'), REQUIRED)['missing'], REQUIRED)

    def test_every_required_release_lane_is_checked(self):
        lanes = ['from/xaa/REL-ALL/**', 'from/xaa/REL-XAB/**']
        self.assertEqual(check(config(lanes[0]), lanes)['missing'], lanes[1:])
        self.assertEqual(check(config(*lanes), lanes)['missing'], [])
        self.assertEqual(check(config('from/xaa/**'), lanes)['missing'], lanes)

    def test_json5_comments_and_acl_strings_are_not_storage_evidence(self):
        text = '''{ // key_expr: "from/xaa/**"
          access_control: {key_expr: "from/xaa/**"},
          plugins: {storage_manager: {storages: {
            history: {key_expr: 'from/**',},
          },},},
        }'''
        self.assertEqual(check(text, REQUIRED)['missing'], REQUIRED)

    def test_quoted_json_keys_and_json5_stores_are_supported(self):
        text = '''{plugins: {storage_manager: {storages: {
          'scoped-store': {'key_expr': 'from/xaa/**',},
        },},},}'''
        self.assertEqual(check(text, REQUIRED)['missing'], [])

    def test_empty_storage_configuration_is_missing(self):
        self.assertEqual(check('{}', REQUIRED)['missing'], REQUIRED)

    def test_disabled_storage_is_not_evidence(self):
        text = '{plugins:{storage_manager:{storages:{disabled:null}}}}'
        self.assertEqual(check(text, REQUIRED)['missing'], REQUIRED)

    def test_malformed_configuration_is_not_verified(self):
        for text in ('{', '[]', '{plugins:[]}', config(None)):
            with self.subTest(text=text):
                report = check(text, REQUIRED)
                self.assertTrue(report['error'] or report['missing'])

    def test_invalid_expected_scopes_are_not_verified(self):
        for scopes in ([], 'from/xaa/**', [None], ['']):
            with self.subTest(scopes=scopes):
                self.assertTrue(check(config(*REQUIRED), scopes)['error'])

    def test_real_template_retains_history_and_satisfies_each_peer(self):
        path = ROOT / 'ops/ansible/roles/goat-zenoh/tests/render-check.py'
        spec = importlib.util.spec_from_file_location('ingress_render_test', path)
        renderer = importlib.util.module_from_spec(spec)
        with contextlib.redirect_stdout(io.StringIO()), unittest.mock.patch('sys.exit'):
            spec.loader.exec_module(renderer)
        self.assertEqual(renderer.fails, 0)
        peers = [{'sovereign': 'xaa', 'cn': 'xaa-peer', 'release_key_exprs': []},
                 {'sovereign': 'zag-ros', 'cn': 'zag-ros-peer', 'release_key_exprs': []}]
        for identity in ('per-vendor', 'per-uuid-slot-pool'):
            text = renderer.render(zenoh_identity_model=identity,
                                   zenoh_sovereign_peerings=peers,
                                   zenoh_from_retention_days=3, zenoh_slot_pool_size=2)
            report = check(text, ['from/' + p['sovereign'] + '/**' for p in peers])
            self.assertEqual(report['error'], '')
            self.assertEqual(report['missing'], [])
            self.assertIn('from/**', report['observed'])


@unittest.skipUnless(shutil.which('ansible-playbook'), 'ansible-playbook is required')
class RolePreflight(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        bindir = self.root / 'bin'
        bindir.mkdir()
        docker = bindir / 'docker'
        docker.write_text('#!/bin/sh\nprintf "invoked\\n" >> "$INGRESS_TRACE"\n'
                          'if [ "$INGRESS_UNREADABLE" = 1 ]; then exit 1; fi\n'
                          'cat "$INGRESS_CONFIG"\n')
        docker.chmod(0o755)
        self.trace = self.root / 'trace'
        self.input = self.root / 'config.json5'
        self.env = {**os.environ, 'PATH': str(bindir) + ':' + os.environ['PATH'],
                    'INGRESS_TRACE': str(self.trace), 'INGRESS_CONFIG': str(self.input),
                    'ANSIBLE_FILTER_PLUGINS': str(PLUGIN.parent),
                    'ANSIBLE_LOCAL_TEMP': str(self.root / 'ansible-local'),
                    'ANSIBLE_REMOTE_TEMP': str(self.root / 'ansible-remote'),
                    'ANSIBLE_NOCOLOR': '1', 'ANSIBLE_STDOUT_CALLBACK': 'default',
                    'OBJC_DISABLE_INITIALIZE_FORK_SAFETY': 'YES'}

    def run_play(self, text, enabled=True, unreadable=False, required=None):
        self.input.write_text(text)
        task = ROOT / 'ops/ansible/roles/goat-zenoh-edge/tasks/ingress-storage-check.yml'
        play = [{'hosts': 'localhost', 'connection': 'local', 'gather_facts': False,
                 'become': False, 'vars': {
                     'ansible_python_interpreter': sys.executable,
                     'goat_zenoh_edge_ingress_scope_check': enabled,
                     'goat_zenoh_edge_ingress_key_exprs': required or REQUIRED},
                 'tasks': [{'ansible.builtin.include_tasks': str(task)}]}]
        path = self.root / 'play.yml'
        path.write_text(yaml.safe_dump(play, sort_keys=False))
        return subprocess.run(['ansible-playbook', '-i', 'localhost,', str(path)],
                              env={**self.env, 'INGRESS_UNREADABLE': '1' if unreadable else '0'},
                              capture_output=True, text=True, timeout=45)

    def test_legacy_plus_scoped_passes_the_real_role_tasks(self):
        result = self.run_play(config('from/**', *REQUIRED))
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn('PASS: a matching receive store', result.stdout)

    def test_legacy_only_fails_the_real_role_tasks(self):
        result = self.run_play(config('from/**'))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('Missing exact scopes: from/xaa/**', result.stdout)

    def test_scoped_only_passes(self):
        result = self.run_play(config(*REQUIRED))
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_missing_one_of_two_lanes_fails(self):
        lanes = ['from/xaa/REL-ALL/**', 'from/xaa/REL-XAB/**']
        result = self.run_play(config(lanes[0]), required=lanes)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(lanes[1], result.stdout)

    def test_unavailable_core_is_explicitly_skipped(self):
        result = self.run_play('', unreadable=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn('SKIP: the local core configuration', result.stdout)
        self.assertNotIn('PASS: a matching receive store', result.stdout)

    def test_disabled_check_skips_without_reading_docker(self):
        result = self.run_play('', enabled=False)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn('SKIP: ingress storage inspection is explicitly disabled', result.stdout)
        self.assertFalse(self.trace.exists())
        self.assertNotIn('PASS: a matching receive store', result.stdout)

    def test_malformed_readable_config_fails_without_logging_secrets(self):
        secret = 'synthetic-private-config-value'
        result = self.run_play('{password:"' + secret + '", broken: }')
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn(secret, result.stdout + result.stderr)
        self.assertIn('Cannot inspect storage declarations', result.stdout)


if __name__ == '__main__':
    unittest.main()
