"""Relay isolation requires a responding public control on the same relay."""
import importlib.util
from pathlib import Path
import subprocess
import sys
import tomllib
import unittest
from unittest.mock import patch
import yaml
from jinja2 import Environment, StrictUndefined

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('relay_isolation_probes', ROOT / 'ops/validation/probes.py')
probes = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = probes
spec.loader.exec_module(probes)
PARAMS = {'host': '192.0.2.10', 'public_name': 'netbird.example.test',
          'protected_targets': [{'name': 'dashboard.zagros.netbird.example.test', 'path': '/'},
                                {'name': 'auth.zagros.netbird.example.test', 'path': '/sessions/whoami'}]}

def response(code, rc=0):
    return subprocess.CompletedProcess([], rc, str(code), 'private diagnostic')

class RelayIsolation(unittest.TestCase):
    def probe(self, replies, params=None):
        with patch.object(probes.shutil, 'which', return_value='/usr/bin/curl'), patch.object(probes.subprocess, 'run', side_effect=replies) as run:
            result = probes.relay_mesh_isolation(params or PARAMS)
        self.assertNotIn('private diagnostic', result.evidence)
        return result, run

    def test_live_enrollment_and_all_protected_routes_refused(self):
        result, run = self.probe([response(401), response(404), response(404)])
        self.assertEqual(result.status, probes.PASS)
        for call in run.call_args_list:
            cmd = call.args[0]
            self.assertIn('netbird.example.test:443:192.0.2.10', cmd)
            self.assertEqual(cmd[cmd.index('--noproxy') + 1], '*')
            self.assertTrue(cmd[-1].startswith('https://netbird.example.test/'))
            self.assertNotIn('-k', cmd)
            self.assertNotIn('--insecure', cmd)
            self.assertNotIn('--location', cmd)
        self.assertIn('Host: auth.zagros.netbird.example.test', run.call_args_list[-1].args[0])
        self.assertEqual(run.call_args_list[-1].args[0][-1], 'https://netbird.example.test/sessions/whoami')

    def test_unreachable_or_tls_failure_never_passes(self):
        for error in (response('000', 7), response('000', 60), subprocess.TimeoutExpired('curl', 8), OSError('no tool')):
            for responses in ([error], [response(401), error]):
                self.assertEqual(self.probe(responses)[0].status, probes.UNKNOWN)

    def test_broken_enrollment_control_fails_before_negative(self):
        for code in (200, 301, 403, 404, 500, 502):
            result, run = self.probe([response(code)])
            self.assertEqual(result.status, probes.FAIL)
            self.assertEqual(run.call_count, 1)

    def test_protected_surface_response_fails_even_when_auth_required(self):
        for code in (200, 302, 401, 403, 421, 500, 502):
            self.assertEqual(self.probe([response(401), response(code)])[0].status, probes.FAIL)

    def test_empty_targets_and_non_ip_relay_are_unknown(self):
        for params in (dict(PARAMS, protected_targets=[]), dict(PARAMS, host='relay.example.test'), dict(PARAMS, public_name='bad name')):
            result, run = self.probe([], params)
            self.assertEqual(result.status, probes.UNKNOWN)
            run.assert_not_called()

    def test_ipv6_relay_is_pinned(self):
        _, run = self.probe([response(401), response(404), response(404)], dict(PARAMS, host='2001:db8::1'))
        self.assertIn('netbird.example.test:443:[2001:db8::1]', run.call_args.args[0])

    def test_batch_requires_each_relay_and_propagates_unknown(self):
        import runpy
        with patch.object(sys, 'path', [str(ROOT / 'ops/validation')] + sys.path):
            runner = runpy.run_path(str(ROOT / 'ops/validation/run-acceptance.py'))
        params = dict(PARAMS, hosts_from='relay_public_ips')
        values = {'relay_public_ips': ['192.0.2.10', '192.0.2.11']}
        for last, want in ((404, probes.PASS), (200, probes.FAIL)):
            replies = [response(401), response(404), response(404), response(401), response(last), response(404)]
            with patch.object(probes.shutil, 'which', return_value='/usr/bin/curl'), patch.object(probes.subprocess, 'run', side_effect=replies) as run:
                result = runner['run_one'](probes.relay_mesh_isolation, 'relay-mesh-isolation', params, {}, values)
            self.assertEqual(result.status, want)
            self.assertTrue(any('netbird.example.test:443:192.0.2.11' in call.args[0] for call in run.call_args_list))
        result = runner['run_one'](lambda p: probes.Verdict(probes.UNKNOWN), 'relay-mesh-isolation', params, {}, values)
        self.assertEqual(result.status, probes.UNKNOWN)
        result = runner['run_one'](probes.relay_mesh_isolation, 'relay-mesh-isolation', params, {}, {})
        self.assertEqual(result.status, probes.UNKNOWN)

    def test_template_defaults_and_override(self):
        template = Environment(undefined=StrictUndefined).from_string((ROOT / 'ops/ansible/templates/client.toml.j2').read_text())
        common = dict(relay_name='fixture', relay_ip='192.0.2.10', relay_token='fixture-token', relay_pubkey='fixture-key')
        # all.yml need not set the key (the template default applies); if it does, only to the enrollment listener.
        shared = yaml.safe_load((ROOT / 'ops/ansible/group_vars/all.yml').read_text()) or {}
        key = 'rathole_client_netbird_mgmt_local_addr'
        self.assertEqual(shared.get(key, '127.0.0.1:8443'), '127.0.0.1:8443')
        for extra, want in (({}, '127.0.0.1:8443'), ({key: '127.0.0.1:8443'}, '127.0.0.1:8443'), ({key: '127.0.0.1:9443'}, '127.0.0.1:9443')):
            rendered = tomllib.loads(template.render(**common, **extra))
            self.assertEqual(rendered['client']['services']['netbird-mgmt']['local_addr'], want)
        compose = yaml.safe_load((ROOT / 'ops/compose/mgmt-host/docker-compose.yml').read_text())
        traefik = compose['services']['traefik']
        self.assertIn('127.0.0.1:8443:8443', traefik['ports'])
        self.assertIn('--entrypoints.enrollment.address=:8443', traefik['command'])

    def test_manifest_checks_each_declared_relay_without_verdict_inversion(self):
        rows = yaml.safe_load((ROOT / 'ops/validation/acceptance-tests.yaml').read_text())['tests']
        row = next(r for r in rows if r['id'] == 'relay-refuses-mesh-only-hosts')
        self.assertIs(probes.REGISTRY[row['probe']], probes.relay_mesh_isolation)
        self.assertEqual(row['params']['hosts_from'], 'relay_public_ips')
        self.assertEqual(row['applies_when'], 'relay_public_ips')  # a never-set var would make it N/A everywhere
        self.assertIn('values["relay_public_ips"] =', (ROOT / 'ops/validation/generate-values.py').read_text())
        self.assertNotIn('requires', row)
        self.assertFalse(row.get('must_refuse', False))

if __name__ == '__main__':
    unittest.main()
