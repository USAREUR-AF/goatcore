"""Phase 8 opens the deployment's age-encrypted CA key itself, into a file the
run owns and removes; an explicit key file still wins. No live CA or deployment.

Most tests put a stand-in `age` on PATH (it "decrypts" by copying, and only
for the identity file that says `good`), so they run wherever OpenSSL does. One
test uses the real `age` when it is installed.
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

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / 'ops/federation/standup/mint-peering-leaves.sh'

FAKE_AGE = r'''#!/usr/bin/env bash
# age -d -i <identity> -o <out> [--] <in>. Records where it wrote, and the
# directory's and file's modes, in $FAKE_AGE_LOG (outside the run's TMPDIR).
set -eu
id=""; out=""; in=""; decrypt=0
while [ $# -gt 0 ]; do
  case "$1" in
    -d) decrypt=1; shift ;;
    -i) id="$2"; shift 2 ;;
    -o) out="$2"; shift 2 ;;
    --) in="$2"; shift 2 ;;
    *) in="$1"; shift ;;
  esac
done
[ "$decrypt" = 1 ] || { echo "age: not asked to decrypt" >&2; exit 1; }
case "$(cat "$id")" in
  good) ;;
  leak) echo "age: error: unexpected intro: \"AGE-SECRET-KEY-1QQQSECRET\"" >&2; exit 1 ;;
  *) echo "age: error: no identity matched any of the recipients" >&2; exit 1 ;;
esac
cp "$in" "$out"
python3 -c 'import os,sys; d=os.path.dirname(sys.argv[1]); print(sys.argv[1], oct(os.stat(d).st_mode & 0o777), oct(os.stat(sys.argv[1]).st_mode & 0o777))' "$out" >> "$FAKE_AGE_LOG"
[ -z "${FAKE_AGE_SLEEP:-}" ] || sleep "$FAKE_AGE_SLEEP"
'''


@unittest.skipUnless(shutil.which('openssl'), 'OpenSSL is required')
class CaKeyFromAge(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.out = self.root / 'leaves'
        self.ca = self.root / 'ca.pem'
        self.key = self.root / 'ca-key.pem'
        subprocess.run(['openssl', 'req', '-x509', '-newkey', 'ec', '-pkeyopt', 'ec_paramgen_curve:prime256v1',
                        '-nodes', '-keyout', str(self.key), '-out', str(self.ca), '-days', '365',
                        '-subj', '/CN=Test-CA', '-addext', 'basicConstraints=critical,CA:TRUE'],
                       capture_output=True, check=True)
        der = subprocess.check_output(['openssl', 'x509', '-in', str(self.ca), '-outform', 'DER'])
        self.agreement = self.root / 'agreement.json'
        self.agreement.write_text(json.dumps({
            'parties': [{'sovereign_id': 'xaa', 'ca_fingerprint': 'sha256:' + hashlib.sha256(der).hexdigest()},
                        {'sovereign_id': 'xab'}],
            'transport': {'link_subnet': '169.254.10.0/31'},
        }))
        # The run's own temporary directory, so a leftover copy of the key shows.
        self.tmp = self.root / 'tmp'
        self.tmp.mkdir()
        bin_dir = self.root / 'bin'
        bin_dir.mkdir()
        (bin_dir / 'age').write_text(FAKE_AGE)
        (bin_dir / 'age').chmod(0o755)
        self.fake_path = f"{bin_dir}{os.pathsep}{os.environ['PATH']}"
        # The stand-in "encrypts" by copying: the .age file holds the PEM.
        self.age_file = self.root / 'ca.key.age'
        shutil.copy(self.key, self.age_file)
        self.good = self.root / 'good-identity.txt'
        self.good.write_text('good')
        self.bad = self.root / 'bad-identity.txt'
        self.bad.write_text('bad')
        self.log = self.root / 'age.log'
        self.log.write_text('')

    def run_mint(self, *extra, env=None, path=None, cwd=None, opt_in=True):
        e = {k: v for k, v in os.environ.items() if not k.startswith('FED_')}
        e.update({'TMPDIR': str(self.tmp), 'FED_CA_KEY_TMP': str(self.tmp), 'PATH': path or self.fake_path,
                  'FAKE_AGE_LOG': str(self.log)})
        if opt_in:
            e['FED_CA_KEY_FROM_AGE'] = '1'
        e.update(env or {})
        args = ['bash', str(SCRIPT), str(self.agreement), str(self.out), '--as', 'xaa',
                '--ca-cert', str(self.ca), '--core-cn', 'router', '--core-san', 'core.example.test', *extra]
        return subprocess.run(args, env=e, capture_output=True, text=True, cwd=cwd)

    def assert_no_key_left(self):
        self.assertEqual(list(self.tmp.iterdir()), [], 'the run left its copy of the CA key behind')

    def assert_written_privately(self):
        """The stand-in wrote the key inside the run's TMPDIR, in a 0700
        directory as a 0600 file, and that file is gone now."""
        lines = self.log.read_text().split('\n')
        written = [l.split() for l in lines if l.strip()]
        self.assertEqual(len(written), 1, self.log.read_text())
        path, dir_mode, file_mode = written[0]
        self.assertTrue(Path(path).is_relative_to(self.tmp), path)
        self.assertEqual((dir_mode, file_mode), ('0o700', '0o600'))
        self.assertFalse(Path(path).exists())

    def test_the_age_key_issues_the_leaves_and_its_copy_is_removed(self):
        r = self.run_mint(env={'FED_CA_KEY_AGE': str(self.age_file), 'FED_AGE_IDENTITY': str(self.good)})
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn('opened', r.stdout)
        self.assertEqual(len(list(self.out.glob('*.pem'))), 9)  # four pairs and our CA
        self.assert_no_key_left()
        self.assert_written_privately()

    def test_off_by_default_nothing_is_decrypted(self):
        r = self.run_mint(env={'FED_CA_KEY_AGE': str(self.age_file), 'FED_AGE_IDENTITY': str(self.good)},
                          opt_in=False)
        self.assertNotEqual(r.returncode, 0, 'issuance needs a key, and none was opened')
        self.assertEqual(self.log.read_text(), '', 'age ran without the opt-in')
        self.assertIn('is not opened (opt-in: FED_CA_KEY_FROM_AGE=1', r.stderr)
        self.assertIn('FED_CA_KEY_FROM_AGE=1', r.stdout + r.stderr)
        self.assert_no_key_left()

    def test_the_flag_opts_in_too(self):
        r = self.run_mint('--ca-key-from-age', env={'FED_CA_KEY_AGE': str(self.age_file),
                                                    'FED_AGE_IDENTITY': str(self.good)}, opt_in=False)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assert_written_privately()

    @unittest.skipUnless(os.path.isdir('/dev/shm') and os.access('/dev/shm', os.W_OK), 'no writable /dev/shm')
    def test_without_a_named_directory_the_key_goes_to_dev_shm(self):
        e = {'FED_CA_KEY_AGE': str(self.age_file), 'FED_AGE_IDENTITY': str(self.good), 'FED_CA_KEY_TMP': ''}
        r = self.run_mint(env=e)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        path = self.log.read_text().split()[0]
        self.assertTrue(path.startswith('/dev/shm/fed-ca-key.'), path)
        self.assertFalse(Path(path).exists())
        self.assertFalse(Path(path).parent.exists())
        self.assert_no_key_left()

    def test_flags_name_the_age_key_too(self):
        r = self.run_mint('--ca-key-age', str(self.age_file), '--age-identity', str(self.good))
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assert_no_key_left()

    def test_an_explicit_key_file_wins(self):
        r = self.run_mint('--ca-key', str(self.key),
                          env={'FED_CA_KEY_AGE': str(self.root / 'absent.age'), 'FED_AGE_IDENTITY': str(self.bad)})
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertNotIn('opened', r.stdout)
        self.assertNotIn('not opened', r.stderr)
        r = self.run_mint(env={'FED_CA_KEY': str(self.key), 'FED_CA_KEY_AGE': str(self.root / 'absent.age')})
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertNotIn('not opened', r.stderr)

    def test_a_wrong_identity_refuses_issuance_and_leaves_nothing(self):
        r = self.run_mint(env={'FED_CA_KEY_AGE': str(self.age_file), 'FED_AGE_IDENTITY': str(self.bad)})
        self.assertNotEqual(r.returncode, 0, r.stdout)
        self.assertIn('CA key not opened', r.stderr)
        self.assertIn('FED_CA_KEY_AGE', r.stdout + r.stderr)
        self.assertFalse(list(self.out.glob('*.pem')) if self.out.exists() else [])
        self.assert_no_key_left()

    def test_a_message_from_age_never_repeats_a_secret(self):
        leak = self.root / 'leak-identity.txt'
        leak.write_text('leak')
        r = self.run_mint(env={'FED_CA_KEY_AGE': str(self.age_file), 'FED_AGE_IDENTITY': str(leak)})
        self.assertNotEqual(r.returncode, 0)
        self.assertIn('CA key not opened', r.stderr)
        self.assertNotIn('QQQSECRET', r.stdout + r.stderr)
        self.assert_no_key_left()

    @unittest.skipUnless(shutil.which('age') and shutil.which('age-keygen'), 'age is not installed')
    def test_a_key_path_that_starts_with_a_dash_is_a_path(self):
        identity = self.root / 'keys.txt'
        subprocess.run(['age-keygen', '-o', str(identity)], capture_output=True, check=True)
        recipient = subprocess.check_output(['age-keygen', '-y', str(identity)], text=True).strip()
        subprocess.run(['age', '-r', recipient, '-o', str(self.root / '-ca.key.age'), str(self.key)], check=True)
        r = self.run_mint(env={'FED_CA_KEY_AGE': '-ca.key.age', 'FED_AGE_IDENTITY': str(identity)},
                          path=os.environ['PATH'], cwd=self.root)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assert_no_key_left()

    def test_termination_removes_the_copy(self):
        e = {k: v for k, v in os.environ.items() if not k.startswith('FED_')}
        e.update({'TMPDIR': str(self.tmp), 'FED_CA_KEY_TMP': str(self.tmp), 'FED_CA_KEY_FROM_AGE': '1',
                  'PATH': self.fake_path, 'FAKE_AGE_LOG': str(self.log), 'FAKE_AGE_SLEEP': '30',
                  'FED_CA_KEY_AGE': str(self.age_file), 'FED_AGE_IDENTITY': str(self.good)})
        args = ['bash', str(SCRIPT), str(self.agreement), str(self.out), '--as', 'xaa', '--ca-cert', str(self.ca)]
        proc = subprocess.Popen(args, env=e, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
        try:
            for _ in range(200):
                if self.log.read_text().strip():
                    break
                time.sleep(0.05)
            self.assertTrue(self.log.read_text().strip(), 'the stand-in never wrote the key')
            os.killpg(proc.pid, signal.SIGTERM)
            self.assertEqual(proc.wait(timeout=20), 143)
        finally:
            if proc.poll() is None:
                os.killpg(proc.pid, signal.SIGKILL)
                proc.wait()
        self.assert_no_key_left()

    def test_a_rerun_that_reuses_every_leaf_needs_no_key(self):
        self.assertEqual(self.run_mint('--ca-key', str(self.key)).returncode, 0)
        r = self.run_mint(env={'FED_CA_KEY_AGE': str(self.root / 'absent.age')})
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn('reused', r.stdout.lower())
        self.assertIn('does not exist', r.stderr)

    def test_a_failed_issuance_still_removes_the_copy(self):
        # An opened key that does not match the CA: issuance refuses, and the
        # copy of the key is removed all the same.
        other = self.root / 'other-key.pem'
        subprocess.run(['openssl', 'genpkey', '-algorithm', 'EC', '-pkeyopt', 'ec_paramgen_curve:prime256v1',
                        '-out', str(other)], capture_output=True, check=True)
        shutil.copy(other, self.age_file)
        r = self.run_mint(env={'FED_CA_KEY_AGE': str(self.age_file), 'FED_AGE_IDENTITY': str(self.good)})
        self.assertNotEqual(r.returncode, 0, r.stdout)
        self.assertIn('opened', r.stdout)
        self.assert_no_key_left()

    @unittest.skipUnless(shutil.which('age') and shutil.which('age-keygen'), 'age is not installed')
    def test_the_real_age_round_trip(self):
        identity = self.root / 'keys.txt'
        subprocess.run(['age-keygen', '-o', str(identity)], capture_output=True, check=True)
        recipient = subprocess.check_output(['age-keygen', '-y', str(identity)], text=True).strip()
        real = self.root / 'real.key.age'
        subprocess.run(['age', '-r', recipient, '-o', str(real), str(self.key)], check=True)
        r = self.run_mint(env={'FED_CA_KEY_AGE': str(real), 'FED_AGE_IDENTITY': str(identity)},
                          path=os.environ['PATH'])
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual(len(list(self.out.glob('*.pem'))), 9)  # four pairs and our CA
        self.assert_no_key_left()


if __name__ == '__main__':
    unittest.main()
