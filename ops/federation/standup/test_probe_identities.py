"""Phase 14's probe identities (docs/design/federation-acceptance.md §2.4).

`mint-peering-leaves.sh --probe <run id>` issues the forged-name and the impersonation
leaf for one acceptance run from our own peering CA; `probe-identities.sh` is the
launcher a side gives phase 14 (FED_PROBE_LAUNCHER_OURS): for each probe it puts the
identity in a private folder on memory-backed storage, hands it to the probe launcher,
and deletes that folder on any exit, checks it is gone and says so.

No live CA and no edge host: the CA is made here, and the probe launcher is a stand-in
that records what it was given.
"""
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import tempfile
import time
import unittest

HERE = Path(__file__).resolve().parent
MINT = HERE / 'mint-peering-leaves.sh'
WRAP = HERE / 'probe-identities.sh'
RUN = 'acc-20261115T190000Z-4242-0a1b2c3d'
TTL = 900

# A probe launcher stand-in: start copies what FED_PL_DIR holds (names and modes) into
# $STUB_LOG, then prints the contract's lines; STUB_START_RC and STUB_START_SLEEP shape it.
STUB = r'''#!/usr/bin/env bash
case "$1" in
  start)
    { echo "dir=$FED_PL_DIR mode=$(stat -c %a "$FED_PL_DIR" 2>/dev/null || stat -f %Lp "$FED_PL_DIR")"
      for f in "$FED_PL_DIR"/*; do
        echo "file=$(basename "$f") mode=$(stat -c %a "$f" 2>/dev/null || stat -f %Lp "$f") sha=$(sha256sum "$f" | cut -c1-64)"
      done; } >> "$STUB_LOG"
    cp "$FED_PL_DIR/$2-cert.pem" "$STUB_SEEN/$2-cert.pem"
    echo "HANDLE=goat-probe-$2-$3"
    [ -z "${STUB_START_SLEEP:-}" ] || sleep "$STUB_START_SLEEP"
    [ "${STUB_START_RC:-0}" = 0 ] || exit "$STUB_START_RC"
    echo "REST=http://192.0.2.10:18600"; echo "ZID=f0123456789abcdef"
    ;;
  stop) echo "stop $2" >> "$STUB_LOG"; echo KEY_DELETED=verified ;;
  alive) echo "alive $2" >> "$STUB_LOG"; exit 1 ;;
esac
'''


def x509(cert, *args):
    return subprocess.run(['openssl', 'x509', '-in', str(cert), '-noout', *args],
                          capture_output=True, text=True, check=True).stdout


def epoch(openssl_date):
    return int(time.mktime(time.strptime(openssl_date.strip(), '%b %d %H:%M:%S %Y GMT'))) - time.timezone


@unittest.skipUnless(shutil.which('openssl'), 'OpenSSL is required')
class ProbeIdentities(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.ca = self.root / 'ca.pem'
        self.ca_key = self.root / 'ca-key.pem'
        subprocess.run(['openssl', 'req', '-x509', '-newkey', 'ec', '-pkeyopt', 'ec_paramgen_curve:prime256v1',
                        '-nodes', '-keyout', str(self.ca_key), '-out', str(self.ca), '-days', '365',
                        '-subj', '/CN=Test-CA', '-addext', 'basicConstraints=critical,CA:TRUE',
                        '-addext', 'keyUsage=critical,keyCertSign,cRLSign'],
                       capture_output=True, check=True)
        der = subprocess.check_output(['openssl', 'x509', '-in', str(self.ca), '-outform', 'DER'])
        self.agreement = self.root / 'agreement.json'
        self.agreement.write_text(json.dumps({
            'parties': [{'sovereign_id': 'xaa', 'ca_fingerprint': 'sha256:' + hashlib.sha256(der).hexdigest()},
                        {'sovereign_id': 'xab'}],
            'transport': {'link_subnet': '169.254.10.0/31'},
        }))
        self.probe = self.root / 'probe'
        self.probe.mkdir(mode=0o700)
        self.probe.chmod(0o700)
        self.record = self.root / 'ca-state' / 'issued-certificates.jsonl'
        self.record.parent.mkdir()
        self.tmp = self.root / 'tmp'
        self.tmp.mkdir()

    # --- the minting mode ---------------------------------------------------------------
    def env(self, extra=None):
        e = {k: v for k, v in os.environ.items() if not k.startswith('FED_')}
        e.update({'TMPDIR': str(self.tmp), 'FED_CA_KEY_TMP': str(self.tmp), 'FED_PROBE_TTL': str(TTL)})
        e.update(extra or {})
        return e

    def mint(self, *extra, out=None, env=None, key=True):
        args = ['bash', str(MINT), str(self.agreement), str(out or self.probe), '--as', 'xaa',
                '--ca-cert', str(self.ca), '--probe', RUN, '--issued-record', str(self.record), *extra]
        if key:
            args += ['--ca-key', str(self.ca_key)]
        return subprocess.run(args, env=self.env(env), capture_output=True, text=True)

    def files(self):
        return sorted(str(p.relative_to(self.root)) for p in self.root.rglob('*') if p.is_file())

    def records(self):
        return [json.loads(l) for l in self.record.read_text().splitlines() if l.strip()]

    def test_the_forged_leaf_has_the_derived_name_and_only_the_probe_extensions(self):
        r = self.mint('--probe-kind', 'forged')
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        cert = self.probe / 'forged-cert.pem'
        self.assertIn('CN=fed.xaa.xab.acceptance-forged.0a1b2c3d', x509(cert, '-subject', '-nameopt', 'compat'))
        text = x509(cert, '-text')
        self.assertIn('CA:FALSE', text)
        self.assertIn('TLS Web Client Authentication', text)
        self.assertNotIn('TLS Web Server Authentication', text)
        eku = text.split('X509v3 Extended Key Usage:')[1].split('\n')[1].strip()
        self.assertEqual(eku, 'TLS Web Client Authentication', 'clientAuth and nothing else')
        ku = text.split('X509v3 Key Usage:')[1].split('\n')[1].strip()
        self.assertEqual(ku, 'Digital Signature')
        self.assertIn('URI:urn:goat:acceptance-probe:' + RUN, text)
        subprocess.run(['openssl', 'verify', '-CAfile', str(self.ca), '-purpose', 'sslclient', str(cert)],
                       capture_output=True, check=True)
        self.assertNotEqual(subprocess.run(['openssl', 'verify', '-CAfile', str(self.ca), '-purpose', 'sslserver',
                                            str(cert)], capture_output=True).returncode, 0,
                            'a probe leaf is never a server certificate')

    def test_the_lifetime_is_the_ttl_and_fifteen_minutes_and_starts_a_minute_back(self):
        now = int(time.time())
        r = self.mint('--probe-kind', 'forged')
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        cert = self.probe / 'forged-cert.pem'
        nb = epoch(x509(cert, '-startdate').split('=', 1)[1])
        na = epoch(x509(cert, '-enddate').split('=', 1)[1])
        self.assertTrue(now - 70 <= nb <= now - 50, (now, nb))
        self.assertTrue(now + TTL + 900 - 10 <= na <= now + TTL + 900 + 10, (now, na))

    def test_the_key_is_private_and_nothing_is_written_outside_the_probe_folder_and_the_record(self):
        before = self.files()
        r = self.mint('--probe-kind', 'forged')
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual(oct((self.probe / 'forged-key.pem').stat().st_mode & 0o777), '0o600')
        added = sorted(set(self.files()) - set(before))
        self.assertEqual(added, ['ca-state/issued-certificates.jsonl', 'probe/forged-cert.pem', 'probe/forged-key.pem'])
        self.assertEqual(list(self.tmp.iterdir()), [])

    def test_no_impersonation_leaf_without_the_flag_naming_this_run(self):
        for extra in ([], ['--impersonate-for-run', 'acc-20261115T180000Z-1-ffffffff'],
                      ['--impersonate-for-run', RUN]):
            with self.subTest(extra=extra):
                r = self.mint('--probe-kind', 'impersonate', '--partner-core-cn', 'router.xab', *extra)
                if extra == ['--impersonate-for-run', RUN]:
                    self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
                    self.assertIn('CN=router.xab', x509(self.probe / 'impersonate-cert.pem', '-subject',
                                                        '-nameopt', 'compat'))
                else:
                    self.assertEqual(r.returncode, 2, r.stdout + r.stderr)
                    self.assertIn('needs --impersonate-for-run ' + RUN, r.stderr)
                    self.assertEqual(list(self.probe.iterdir()), [])
                    self.assertFalse(self.record.exists())

    def test_the_partner_core_name_is_never_defaulted(self):
        r = self.mint('--probe-kind', 'impersonate', '--impersonate-for-run', RUN)
        self.assertEqual(r.returncode, 2, r.stdout + r.stderr)
        self.assertIn('--partner-core-cn', r.stderr)
        self.assertEqual(list(self.probe.iterdir()), [])

    def test_a_name_in_our_prefix_or_the_flag_without_the_impersonation_kind_is_refused(self):
        r = self.mint('--probe-kind', 'forged', '--impersonate-for-run', RUN)
        self.assertEqual(r.returncode, 2, r.stdout + r.stderr)
        self.assertIn('only for --probe-kind impersonate', r.stderr)
        r = self.mint('--probe-kind', 'impersonate', '--impersonate-for-run', RUN,
                      '--partner-core-cn', 'fed.xaa.xab.01')
        self.assertEqual(r.returncode, 2, 'the impersonation leaf names the partner, never one of ours')
        self.assertIn('our own prefix', r.stderr)
        self.assertEqual(list(self.probe.iterdir()), [])

    def test_each_leaf_is_recorded_with_its_run_and_certificate_facts_only(self):
        self.assertEqual(self.mint('--probe-kind', 'forged').returncode, 0)
        second = self.root / 'probe2'
        second.mkdir(mode=0o700)
        second.chmod(0o700)
        r = self.mint('--probe-kind', 'impersonate', '--impersonate-for-run', RUN, '--partner-core-cn', 'router.xab',
                      out=second)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        recs = self.records()
        self.assertEqual([x['kind'] for x in recs], ['forged', 'impersonate'])
        for rec, kind, folder in zip(recs, ('forged', 'impersonate'), (self.probe, second)):
            cert = folder / f'{kind}-cert.pem'
            der = subprocess.check_output(['openssl', 'x509', '-in', str(cert), '-outform', 'DER'])
            self.assertEqual(rec['sha256'], hashlib.sha256(der).hexdigest())
            self.assertEqual(rec['serial'].lower(), x509(cert, '-serial').split('=', 1)[1].strip().lower())
            self.assertEqual(rec['run'], RUN)
            self.assertEqual(rec['not_after'], epoch(x509(cert, '-enddate').split('=', 1)[1]))
            self.assertEqual(set(rec), {'run', 'kind', 'cn', 'serial', 'sha256', 'not_before', 'not_after',
                                        'issued_at', 'issuer_sha256'})
        self.assertEqual(recs[1]['cn'], 'router.xab')
        self.assertNotIn('PRIVATE', self.record.read_text())

    def test_a_probe_leaf_never_reaches_a_trust_file(self):
        leaves = self.root / 'leaves'
        r = subprocess.run(['bash', str(MINT), str(self.agreement), str(leaves), '--as', 'xaa', '--ca-cert',
                            str(self.ca), '--ca-key', str(self.ca_key), '--core-san', 'core.example.test'],
                           env=self.env(), capture_output=True, text=True)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual(self.mint('--probe-kind', 'forged').returncode, 0)
        probe_pem = (self.probe / 'forged-cert.pem').read_text().strip().splitlines()[1]
        for p in leaves.rglob('*'):
            if p.is_file():
                self.assertNotIn(probe_pem, p.read_text(errors='replace'), p)
        # and the probe mode refuses to write into a folder of leaves (or any folder not empty)
        r = self.mint('--probe-kind', 'forged', out=leaves)
        self.assertEqual(r.returncode, 2, r.stdout + r.stderr)

    def test_the_probe_folder_must_be_private(self):
        self.probe.chmod(0o755)
        r = self.mint('--probe-kind', 'forged')
        self.assertEqual(r.returncode, 2, r.stdout + r.stderr)
        self.assertIn('0700', r.stderr)
        self.assertEqual(list(self.probe.iterdir()), [])

    def test_without_a_record_or_a_ca_key_nothing_is_issued(self):
        r = subprocess.run(['bash', str(MINT), str(self.agreement), str(self.probe), '--as', 'xaa', '--ca-cert',
                            str(self.ca), '--ca-key', str(self.ca_key), '--probe', RUN, '--probe-kind', 'forged'],
                           env=self.env(), capture_output=True, text=True)
        self.assertEqual(r.returncode, 2, r.stdout + r.stderr)
        self.assertIn('--issued-record', r.stderr)
        r = self.mint('--probe-kind', 'forged', key=False)
        self.assertNotEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual(list(self.probe.iterdir()), [])

    # --- the wrapper phase 14 calls --------------------------------------------------------
    def wrapper_env(self, extra=None):
        bin_dir = self.root / 'bin'
        bin_dir.mkdir(exist_ok=True)
        stub = bin_dir / 'launcher'
        stub.write_text(STUB)
        stub.chmod(0o755)
        self.seen = self.root / 'seen'
        self.seen.mkdir(exist_ok=True)
        self.log = self.root / 'stub.log'
        pki = self.root / 'pki'
        pki.mkdir(exist_ok=True)
        (pki / 'peering-outward.pem').write_text('OUTWARD CERT\n')
        (pki / 'peering-outward-key.pem').write_text('OUTWARD KEY\n')
        (pki / 'peering-outward-key.pem').chmod(0o600)
        far = self.root / 'far-ca.pem'
        far.write_text('FAR CA\n')
        self.shm = self.root / 'shm'
        self.shm.mkdir(exist_ok=True)
        e = self.env({'FED_CA_KEY_TMP': str(self.shm), 'FED_PROBE_PL': str(stub), 'FED_PROBE_AGREEMENT': str(self.agreement),
                      'FED_PROBE_AS': 'xaa', 'FED_PROBE_PKI': str(pki), 'FED_PROBE_FAR_CA': str(far),
                      'FED_PROBE_ISSUED_RECORD': str(self.record), 'FED_PROBE_PARTNER_CORE_CN': 'router.xab',
                      'FED_CA_CERT': str(self.ca), 'FED_CA_KEY': str(self.ca_key),
                      'STUB_LOG': str(self.log), 'STUB_SEEN': str(self.seen)})
        e.update(extra or {})
        return e

    def wrap(self, *args, env=None):
        return subprocess.run(['bash', str(WRAP), *args], env=self.wrapper_env(env), capture_output=True, text=True)

    def assert_driver_copy_gone(self, out):
        self.assertEqual(list(self.shm.iterdir()), [], 'the driver copy of the probe identity is still there')
        self.assertEqual([l for l in out.splitlines() if l.startswith('KEY_DELETED_DRIVER=')],
                         ['KEY_DELETED_DRIVER=verified'], out)

    def test_agreed_uses_the_outward_leaf_in_a_private_folder_deleted_after_start(self):
        r = self.wrap('start', 'agreed', RUN, str(int(time.time()) + 60))
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn('HANDLE=goat-probe-agreed-' + RUN, r.stdout)
        self.assertIn('REST=http://192.0.2.10:18600', r.stdout)
        log = self.log.read_text()
        self.assertIn('dir=%s/' % self.shm, log)
        self.assertIn('mode=700', log.splitlines()[0])
        self.assertIn('file=agreed-key.pem mode=600', log)
        self.assertIn('file=far-ca.pem', log)
        self.assertEqual((self.seen / 'agreed-cert.pem').read_text(), 'OUTWARD CERT\n')
        self.assert_driver_copy_gone(r.stdout)
        self.assertTrue((self.root / 'pki' / 'peering-outward-key.pem').exists(), 'the live outward key stays')

    def test_forged_and_impersonate_are_minted_for_the_run_and_their_driver_copies_deleted(self):
        for kind in ('forged', 'impersonate'):
            with self.subTest(kind=kind):
                r = self.wrap('start', kind, RUN, str(int(time.time()) + 60))
                self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
                self.assertIn('file=%s-key.pem mode=600' % kind, self.log.read_text())
                subj = x509(self.seen / f'{kind}-cert.pem', '-subject', '-nameopt', 'compat')
                self.assertIn('router.xab' if kind == 'impersonate' else 'acceptance-forged', subj)
                self.assert_driver_copy_gone(r.stdout)
        self.assertEqual([x['kind'] for x in self.records()], ['forged', 'impersonate'])

    def test_a_start_that_fails_still_deletes_the_driver_copy(self):
        r = self.wrap('start', 'forged', RUN, str(int(time.time()) + 60), env={'STUB_START_RC': '1'})
        self.assertEqual(r.returncode, 1, r.stdout + r.stderr)
        self.assert_driver_copy_gone(r.stdout)

    def test_an_interrupted_start_deletes_the_driver_copy(self):
        p = subprocess.Popen(['bash', str(WRAP), 'start', 'forged', RUN, str(int(time.time()) + 60)],
                             env=self.wrapper_env({'STUB_START_SLEEP': '30'}), stdout=subprocess.PIPE,
                             stderr=subprocess.PIPE, text=True)
        for _ in range(100):
            if self.log.exists() and 'file=forged-key.pem' in self.log.read_text():
                break
            time.sleep(0.1)
        self.assertEqual(len(list(self.shm.iterdir())), 1, 'the copy exists while the start runs')
        p.send_signal(signal.SIGTERM)
        out, err = p.communicate(timeout=20)
        self.assertEqual(p.returncode, 143, out + err)
        self.assert_driver_copy_gone(out)

    def test_a_copy_that_could_not_be_deleted_is_reported_and_the_start_fails(self):
        bin_dir = self.root / 'rmbin'
        bin_dir.mkdir()
        (bin_dir / 'rm').write_text('#!/bin/sh\nfor a; do case "$a" in *goat-probe-ids.*) exit 0 ;; esac; done\n'
                                    'exec /bin/rm "$@"\n')
        (bin_dir / 'rm').chmod(0o755)
        r = self.wrap('start', 'agreed', RUN, str(int(time.time()) + 60),
                      env={'PATH': f"{bin_dir}{os.pathsep}{os.environ['PATH']}"})
        self.assertEqual(r.returncode, 1, r.stdout + r.stderr)
        self.assertIn('KEY_DELETED_DRIVER=failed', r.stdout)
        self.assertIn('HANDLE=', r.stdout, 'the handle still reaches the caller, so the probe is stopped')

    def test_a_folder_not_on_memory_backed_storage_is_refused(self):
        r = self.wrap('start', 'agreed', RUN, str(int(time.time()) + 60),
                      env={'FED_CA_KEY_TMP': '', 'FED_PROBE_SHM': str(self.root / 'no-such-shm')})
        self.assertEqual(r.returncode, 2, r.stdout + r.stderr)
        self.assertIn('memory-backed', r.stderr)
        self.assertFalse(self.log.exists(), 'the launcher was not called')

    def test_stop_and_alive_pass_through(self):
        r = self.wrap('stop', 'goat-probe-forged-x')
        self.assertEqual((r.returncode, r.stdout.strip()), (0, 'KEY_DELETED=verified'))
        r = self.wrap('alive', 'goat-probe-forged-x')
        self.assertEqual(r.returncode, 1)
        self.assertEqual(self.log.read_text().split('\n')[:2], ['stop goat-probe-forged-x', 'alive goat-probe-forged-x'])

    def test_a_missing_setting_is_refused_by_name(self):
        for var, kind in (('FED_PROBE_AGREEMENT', 'forged'), ('FED_PROBE_PKI', 'agreed'), ('FED_PROBE_FAR_CA', 'forged'),
                          ('FED_PROBE_ISSUED_RECORD', 'forged')):
            with self.subTest(var=var):
                r = self.wrap('start', kind, RUN, str(int(time.time()) + 60), env={var: ''})
                self.assertEqual(r.returncode, 2, r.stdout + r.stderr)
                self.assertIn(var, r.stderr)
        r = self.wrap('start', 'impersonate', RUN, str(int(time.time()) + 60), env={'FED_PROBE_PARTNER_CORE_CN': ''})
        self.assertEqual(r.returncode, 2, r.stdout + r.stderr)
        self.assertIn('FED_PROBE_PARTNER_CORE_CN', r.stderr)
        self.assertEqual(list(self.shm.iterdir()), [])


if __name__ == '__main__':
    unittest.main()
