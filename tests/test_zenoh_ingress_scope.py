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
        # `docker ps` answers from INGRESS_PS (core discovery); anything else
        # is the config read.
        docker.write_text('#!/bin/sh\n'
                          'if [ "$1" = ps ]; then printf "$INGRESS_PS"; exit 0; fi\n'
                          'printf "invoked %s\\n" "$*" >> "$INGRESS_TRACE"\n'
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

    def run_play(self, text, enabled=True, unreadable=False, required=None,
                 ps='coreid\\tsample-zenoh-router\\t\\n', container=None):
        self.input.write_text(text)
        task = ROOT / 'ops/ansible/roles/goat-zenoh-edge/tasks/ingress-storage-check.yml'
        play = [{'hosts': 'localhost', 'connection': 'local', 'gather_facts': False,
                 'become': False, 'vars': {
                     'ansible_python_interpreter': sys.executable,
                     'goat_zenoh_edge_ingress_scope_check': enabled,
                     'goat_zenoh_edge_ingress_key_exprs': required or REQUIRED,
                     **({'goat_zenoh_edge_core_container': container} if container else {})},
                 'tasks': [{'ansible.builtin.include_tasks': str(task)}]}]
        path = self.root / 'play.yml'
        path.write_text(yaml.safe_dump(play, sort_keys=False))
        return subprocess.run(['ansible-playbook', '-i', 'localhost,', str(path)],
                              env={**self.env, 'INGRESS_UNREADABLE': '1' if unreadable else '0',
                                   'INGRESS_PS': ps},
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

    def test_unavailable_core_configuration_fails(self):
        # Could not run is not a pass: the edge would admit the scopes unverified.
        result = self.run_play('', unreadable=True)
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn('could not read /etc/zenoh/config.json5 from sample-zenoh-router on localhost',
                      result.stdout)
        self.assertNotIn('PASS: a matching receive store', result.stdout)

    def test_disabled_check_skips_without_reading_docker(self):
        result = self.run_play('', enabled=False)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn('SKIP: ingress storage inspection is explicitly disabled', result.stdout)
        self.assertFalse(self.trace.exists())
        self.assertNotIn('PASS: a matching receive store', result.stdout)

    def test_malformed_readable_config_fails_without_logging_raw_config(self):
        # A non-sensitive marker detects raw-config disclosure without putting
        # a credential-shaped value into the on-disk fixture.
        marker = 'synthetic-config-marker-must-not-appear'
        result = self.run_play('{diagnostic_marker:"' + marker + '", broken: }')
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn(marker, result.stdout + result.stderr)
        self.assertIn('Cannot inspect storage declarations', result.stdout)

    def test_the_standard_core_name_is_discovered_and_read(self):
        result = self.run_play(config(*REQUIRED))
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn('core router: sample-zenoh-router', result.stdout)
        self.assertIn('exec sample-zenoh-router cat', self.trace.read_text())

    def test_the_role_label_and_legacy_name_are_discovered(self):
        for row, name in (('cid\\tcustom-core\\tzenoh-core\\n', 'custom-core'),
                          ('lid\\tgoat-zenoh\\t\\n', 'goat-zenoh')):
            result = self.run_play(config(*REQUIRED),
                                   ps='eid\\tgoat-zenoh-edge-sample\\t\\n' + row)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertIn('core router: ' + name, result.stdout)

    def test_no_core_router_fails_as_not_run(self):
        result = self.run_play(config(*REQUIRED), ps='eid\\tgoat-zenoh-edge-sample\\t\\n')
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn('could not run', result.stdout)
        self.assertIn('no core router found', result.stdout)
        self.assertFalse(self.trace.exists())
        self.assertNotIn('PASS: a matching receive store', result.stdout)

    def test_several_core_routers_fail_as_ambiguous(self):
        result = self.run_play(config(*REQUIRED),
                               ps='a\\tsample-zenoh-router\\t\\nb\\tgoat-zenoh\\t\\n')
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn('ambiguous: sample-zenoh-router, goat-zenoh', result.stdout)
        self.assertFalse(self.trace.exists())
        self.assertNotIn('PASS: a matching receive store', result.stdout)

    def test_an_explicit_container_wins_over_discovery(self):
        result = self.run_play(config(*REQUIRED), container='chosen-core',
                               ps='a\\tsample-zenoh-router\\t\\nb\\tgoat-zenoh\\t\\n')
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn('exec chosen-core cat', self.trace.read_text())


@unittest.skipUnless(shutil.which('ansible-playbook'), 'ansible-playbook is required')
class CoreOnAnotherHost(unittest.TestCase):
    """The deployed shape: the edge and the core router on separate hosts.

    Both inventory hosts connect locally; each one's Python interpreter is a
    wrapper that names the host to a fake docker, so the core host runs a core
    router and the edge host runs only the edge. The check must read the core's
    configuration on the core host, the one in <partner>_core.
    """

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        bindir = self.root / 'bin'
        bindir.mkdir()
        docker = bindir / 'docker'
        docker.write_text(
            '#!/bin/sh\n'
            'if [ "$1" = ps ]; then\n'
            '  if [ "$INGRESS_HOST" = core ]; then printf "$INGRESS_CORE_PS"; fi\n'
            '  printf "eid\\tgoat-zenoh-edge-xab\\t\\n"; exit 0\n'
            'fi\n'
            'printf "%s %s\\n" "$INGRESS_HOST" "$*" >> "$INGRESS_TRACE"\n'
            '[ "$INGRESS_HOST" = core ] && [ "$INGRESS_UNREADABLE" != 1 ] || exit 1\n'
            'cat "$INGRESS_CONFIG"\n')
        docker.chmod(0o755)
        for host in ('edge', 'core'):
            wrapper = self.root / ('python-' + host)
            wrapper.write_text('#!/bin/sh\nINGRESS_HOST=' + host + ' PATH="' + str(bindir)
                               + ':$PATH" exec "' + sys.executable + '" "$@"\n')
            wrapper.chmod(0o755)
        self.trace = self.root / 'trace'
        self.input = self.root / 'config.json5'
        self.env = {**os.environ, 'PATH': str(bindir) + ':' + os.environ['PATH'],
                    'INGRESS_TRACE': str(self.trace), 'INGRESS_CONFIG': str(self.input),
                    'ANSIBLE_FILTER_PLUGINS': str(PLUGIN.parent),
                    'ANSIBLE_LOCAL_TEMP': str(self.root / 'ansible-local'),
                    'ANSIBLE_REMOTE_TEMP': str(self.root / 'ansible-remote'),
                    'ANSIBLE_NOCOLOR': '1', 'ANSIBLE_STDOUT_CALLBACK': 'default',
                    'ANSIBLE_RETRY_FILES_ENABLED': '0',
                    'OBJC_DISABLE_INITIALIZE_FORK_SAFETY': 'YES'}

    def inventory(self, cores=('core-1',)):
        def host(name, kind):
            return (name + ' ansible_connection=local ansible_python_interpreter='
                    + str(self.root / ('python-' + kind)))
        lines = ['[xab_edge]', host('edge-1', 'edge'), '', '[xab_core]']
        lines += [host(name, 'edge' if name == 'edge-1' else 'core') for name in cores]
        lines += ['', '[peering_edges:children]', 'xab_edge', '',
                  '[peering_cores:children]', 'xab_core', '']
        path = self.root / 'peering-edges.ini'
        path.write_text('\n'.join(lines))
        return path

    def run_play(self, text, cores=('core-1',), unreadable=False,
                 core_ps='cid\\tgoat-zenoh-router\\tzenoh-core\\n'):
        self.input.write_text(text)
        task = ROOT / 'ops/ansible/roles/goat-zenoh-edge/tasks/ingress-storage-check.yml'
        play = [{'hosts': 'xab_edge', 'gather_facts': False, 'become': False, 'vars': {
                     'goat_zenoh_edge_partner_id': 'xab',
                     'goat_zenoh_edge_ingress_scope_check': True,
                     'goat_zenoh_edge_ingress_key_exprs': REQUIRED},
                 'tasks': [{'ansible.builtin.include_tasks': str(task)}]}]
        path = self.root / 'play.yml'
        path.write_text(yaml.safe_dump(play, sort_keys=False))
        # --limit to the edge group, as the driver runs phase 12.
        return subprocess.run(['ansible-playbook', '-i', str(self.inventory(cores)),
                               '--limit', 'xab_edge', str(path)],
                              env={**self.env, 'INGRESS_CORE_PS': core_ps,
                                   'INGRESS_UNREADABLE': '1' if unreadable else '0'},
                              capture_output=True, text=True, timeout=60)

    def test_broad_store_on_a_separate_core_host_is_refused(self):
        result = self.run_play(config('from/**'))
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn('Missing exact scopes: from/xaa/**', result.stdout)
        self.assertIn('A broad from/** store alone does not satisfy', result.stdout)
        self.assertEqual(self.trace.read_text(), 'core exec goat-zenoh-router cat /etc/zenoh/config.json5\n')

    def test_matching_store_on_a_separate_core_host_passes(self):
        result = self.run_play(config('from/**', *REQUIRED))
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn('core router: goat-zenoh-router on core-1', result.stdout)
        self.assertIn('PASS: a matching receive store', result.stdout)

    def test_no_core_host_in_the_inventory_fails(self):
        result = self.run_play(config(*REQUIRED), cores=())
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn('could not run', result.stdout)
        self.assertIn('no host in inventory group xab_core', result.stdout)
        self.assertFalse(self.trace.exists())
        self.assertNotIn('PASS: a matching receive store', result.stdout)

    def test_several_core_hosts_fail_as_ambiguous(self):
        result = self.run_play(config(*REQUIRED), cores=('core-1', 'core-2'))
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn('inventory group xab_core holds several hosts: core-1, core-2', result.stdout)
        self.assertFalse(self.trace.exists())

    def test_no_core_router_on_the_core_host_fails(self):
        result = self.run_play(config(*REQUIRED), core_ps='')
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn('could not run', result.stdout)
        self.assertIn('on core-1: no core router found', result.stdout)
        self.assertNotIn('PASS: a matching receive store', result.stdout)

    def test_unreadable_core_configuration_on_the_core_host_fails(self):
        result = self.run_play(config(*REQUIRED), unreadable=True)
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn('could not read /etc/zenoh/config.json5 from goat-zenoh-router on core-1',
                      result.stdout)
        self.assertNotIn('PASS: a matching receive store', result.stdout)

    def test_core_group_holding_only_this_host_reads_locally(self):
        # Edge and core on one host, listed in both groups: the local path, once.
        result = self.run_play(config(*REQUIRED), cores=('edge-1',))
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn('on edge-1: no core router found', result.stdout)


if __name__ == '__main__':
    unittest.main()
