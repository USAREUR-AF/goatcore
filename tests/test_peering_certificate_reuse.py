"""Real OpenSSL regression tests for phase-8 reruns; no live CA or deployment."""
import hashlib
import fcntl
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest.mock

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / 'ops/federation/standup/mint-peering-leaves.sh'


@unittest.skipUnless(shutil.which('openssl'), 'OpenSSL is required')
class CertificateReuse(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.out = self.root / 'leaves'
        self.ca = self.root / 'ca.pem'
        self.key = self.root / 'ca-key.pem'
        self.make_ca(self.ca, self.key)
        self.agreement = self.root / 'agreement.json'
        self.agreement.write_text(json.dumps({
            'parties': [{'sovereign_id': 'xaa', 'ca_fingerprint': 'sha256:' + hashlib.sha256(
                subprocess.check_output(['openssl', 'x509', '-in', str(self.ca), '-outform', 'DER'])
            ).hexdigest()}, {'sovereign_id': 'xab'}],
            'transport': {'link_subnet': '169.254.10.0/31'},
        }))
        self.env = {k: v for k, v in os.environ.items() if not k.startswith('FED_')}
        self.args = ['bash', str(SCRIPT), str(self.agreement), str(self.out),
                     '--as', 'xaa', '--ca-cert', str(self.ca),
                     '--core-cn', 'router', '--core-san', 'core.example.test']

    def command(self, args):
        return subprocess.run(args, capture_output=True, text=True, check=True)

    def make_ca(self, cert, key):
        self.command(['openssl', 'req', '-x509', '-newkey', 'ec',
                      '-pkeyopt', 'ec_paramgen_curve:prime256v1', '-nodes',
                      '-keyout', str(key), '-out', str(cert), '-days', '365',
                      '-subj', '/CN=Test-CA', '-addext', 'basicConstraints=critical,CA:TRUE'])

    def invoke(self, *extra, with_key=True, ok=True):
        args = self.args + (['--ca-key', str(self.key)] if with_key else []) + list(extra)
        result = subprocess.run(args, env=self.env, capture_output=True, text=True)
        if ok:
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        else:
            self.assertNotEqual(result.returncode, 0, result.stdout)
        return result

    def snapshot(self):
        return {p.name: (hashlib.sha256(p.read_bytes()).hexdigest(),
                         p.stat().st_mtime_ns, p.stat().st_mode & 0o777)
                for p in self.out.glob('*.pem')}

    def test_rerun_reuses_all_leaves_without_signing_key(self):
        self.invoke()
        before = self.snapshot()
        self.assertEqual(len(before), 8)
        result = self.invoke(with_key=False)
        self.assertEqual(before, self.snapshot())
        self.assertIn('reused', result.stdout.lower())

    def test_rerun_with_signing_key_is_also_a_noop(self):
        self.invoke()
        before = self.snapshot()
        self.invoke()
        self.assertEqual(before, self.snapshot())

    def test_partial_pair_refuses_without_touching_other_leaves(self):
        self.invoke()
        (self.out / 'core-key.pem').unlink()
        before = self.snapshot()
        self.invoke(ok=False)
        self.assertEqual(before, self.snapshot())

    def test_mismatched_key_refuses_without_touching_other_leaves(self):
        self.invoke()
        shutil.copyfile(self.out / 'peering-inward-key.pem', self.out / 'core-key.pem')
        before = self.snapshot()
        self.invoke(ok=False)
        self.assertEqual(before, self.snapshot())

    def test_changed_san_refuses_before_any_leaf_is_replaced(self):
        self.invoke()
        before = self.snapshot()
        self.invoke('--core-san', 'other.example.test', ok=False)
        self.assertEqual(before, self.snapshot())

    def test_different_ca_with_same_subject_is_refused(self):
        self.invoke()
        before = self.snapshot()
        other_ca, other_key = self.root / 'other.pem', self.root / 'other-key.pem'
        self.make_ca(other_ca, other_key)
        self.invoke('--ca-cert', str(other_ca), '--ca-key', str(other_key), ok=False)
        self.assertEqual(before, self.snapshot())

    def test_insufficient_lifetime_requires_explicit_action(self):
        self.invoke()
        before = self.snapshot()
        self.invoke('--min-valid-seconds', str(91 * 86400), ok=False)
        self.assertEqual(before, self.snapshot())

    def test_explicit_rotation_replaces_keys_and_retains_previous_set(self):
        self.invoke()
        before = self.snapshot()
        self.invoke('--rotate')
        after = self.snapshot()
        for name in before:
            self.assertNotEqual(before[name][0], after[name][0], name)
        backups = list(self.out.glob('.previous-*'))
        self.assertEqual(len(backups), 1)
        self.assertEqual({p.name for p in backups[0].glob('*.pem')}, set(before))
        self.invoke(with_key=False)

    def test_missing_leaf_is_issued_without_replacing_valid_pairs(self):
        self.invoke()
        (self.out / 'republisher.pem').unlink()
        (self.out / 'republisher-key.pem').unlink()
        before = self.snapshot()
        self.invoke()
        for name, details in before.items():
            self.assertEqual(details, self.snapshot()[name], name)

    def replace_core_certificate(self, cn='router', eku='serverAuth,clientAuth', days='90', ca=None, ca_key=None):
        csr, ext = self.root / 'replacement.csr', self.root / 'replacement.ext'
        self.command(['openssl', 'req', '-new', '-key', str(self.out / 'core-key.pem'),
                      '-out', str(csr), '-subj', '/CN=' + cn])
        ext.write_text('subjectAltName=DNS:router,DNS:core.example.test\n'
                       'basicConstraints=CA:FALSE\nkeyUsage=digitalSignature,keyEncipherment\n'
                       'extendedKeyUsage=' + eku + '\n')
        if days == 'expired':
            database = self.root / 'index.txt'
            database.write_text('')
            serial = self.root / 'serial'
            serial.write_text('01\n')
            config = self.root / 'ca.conf'
            config.write_text('[ca]\ndefault_ca=test\n[test]\n'
                              f'database={database}\nserial={serial}\n'
                              f'new_certs_dir={self.root}\ncertificate={self.ca}\n'
                              f'private_key={self.key}\ndefault_md=sha256\n'
                              'policy=names\nx509_extensions=leaf\n[names]\n'
                              'commonName=supplied\n[leaf]\n' + ext.read_text())
            self.command(['openssl', 'ca', '-batch', '-notext', '-config', str(config),
                          '-in', str(csr), '-out', str(self.out / 'core.pem'),
                          '-startdate', '20200101000000Z', '-enddate', '20200102000000Z'])
        else:
            self.command(['openssl', 'x509', '-req', '-in', str(csr), '-CA', str(ca or self.ca),
                          '-CAkey', str(ca_key or self.key), '-set_serial', '123', '-days', days,
                          '-extfile', str(ext), '-out', str(self.out / 'core.pem')])


    def test_foreign_issuer_with_same_subject_is_refused(self):
        self.invoke()
        other_ca, other_key = self.root / 'other.pem', self.root / 'other-key.pem'
        self.make_ca(other_ca, other_key)
        self.replace_core_certificate(ca=other_ca, ca_key=other_key)
        before = self.snapshot()
        self.invoke(ok=False)
        self.assertEqual(before, self.snapshot())

    def test_wrong_identity_is_refused(self):
        self.invoke()
        self.replace_core_certificate(cn='unexpected')
        before = self.snapshot()
        self.invoke(ok=False)
        self.assertEqual(before, self.snapshot())

    def test_missing_client_eku_is_refused(self):
        self.invoke()
        self.replace_core_certificate(eku='serverAuth')
        before = self.snapshot()
        self.invoke(ok=False)
        self.assertEqual(before, self.snapshot())

    def test_expired_certificate_is_refused(self):
        self.invoke()
        self.replace_core_certificate(days='expired')
        before = self.snapshot()
        self.invoke(ok=False)
        self.assertEqual(before, self.snapshot())

    def test_concurrent_run_refuses_without_mutation(self):
        self.invoke()
        before = self.snapshot()
        with (self.out / '.mint.lock').open('w') as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            self.invoke('--rotate', ok=False)
        self.assertEqual(before, self.snapshot())

    def test_insecure_key_permissions_are_refused(self):
        self.invoke()
        (self.out / 'core-key.pem').chmod(0o644)
        before = self.snapshot()
        self.invoke(ok=False)
        self.assertEqual(before, self.snapshot())

    def test_wrong_ca_key_cannot_partially_rotate_the_set(self):
        self.invoke()
        other_ca, other_key = self.root / 'other.pem', self.root / 'other-key.pem'
        self.make_ca(other_ca, other_key)
        before = self.snapshot()
        self.invoke('--rotate', '--ca-key', str(other_key), ok=False)
        self.assertEqual(before, self.snapshot())
        self.assertFalse(list(self.out.glob('.previous-*')))

    def test_interrupted_rotation_marker_requires_operator_recovery(self):
        self.invoke()
        before = self.snapshot()
        (self.out / '.rotation-pending').write_text('{}')
        self.invoke(ok=False)
        self.invoke('--rotate', ok=False)
        self.assertEqual(before, self.snapshot())

    def test_install_error_restores_the_previous_set(self):
        self.invoke()
        before = self.snapshot()
        path = ROOT / 'ops/federation/standup/peering_leaf_material.py'
        spec = importlib.util.spec_from_file_location('peering_leaf_material', path)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        fingerprint = json.loads(self.agreement.read_text())['parties'][0]['ca_fingerprint']
        args = [str(path), '--out', str(self.out), '--ca-cert', str(self.ca),
                '--ca-key', str(self.key), '--ca-fingerprint', fingerprint, '--rotate']
        for name, cn, san in [
            ('peering-outward', 'fed.xaa.xab.01', 'DNS:fed.xaa.xab.01,IP:169.254.10.0'),
            ('peering-inward', 'fed.xab.xaa.01', 'DNS:fed.xab.xaa.01'),
            ('republisher', 'fed.xaa.xab.republisher.01', 'DNS:fed.xaa.xab.republisher.01'),
            ('core', 'router', 'DNS:router,DNS:core.example.test'),
        ]:
            args += ['--leaf', name, cn, san]
        original_replace = os.replace
        calls = 0

        def interrupted_replace(source, target):
            nonlocal calls
            calls += 1
            if calls == 2:
                raise OSError('synthetic install failure')
            return original_replace(source, target)

        with unittest.mock.patch('sys.argv', args), unittest.mock.patch.object(module.os, 'replace', interrupted_replace):
            with self.assertRaises(OSError):
                module.main()
        self.assertEqual(before, self.snapshot())

    def test_symlink_is_refused_even_for_rotation(self):
        self.invoke()
        target = self.root / 'external-key'
        shutil.copyfile(self.out / 'core-key.pem', target)
        (self.out / 'core-key.pem').unlink()
        (self.out / 'core-key.pem').symlink_to(target)
        before = target.read_bytes()
        self.invoke('--rotate', ok=False)
        self.assertEqual(target.read_bytes(), before)


if __name__ == '__main__':
    unittest.main()
