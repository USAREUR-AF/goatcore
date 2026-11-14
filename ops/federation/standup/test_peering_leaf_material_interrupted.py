"""An interrupted phase-8 mint leaves no private key behind.

A run killed while minting (kill -9: no cleanup handler runs) used to leave its
private staging directory, pki/.mint-*, with the half-minted keys in it. The
next run neither removed nor ignored it, so the output directory never again
had the shape a clean run gives it. Real OpenSSL; no live CA or deployment.
"""
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import unittest

HERE = Path(__file__).resolve().parent
SCRIPT = HERE / 'mint-peering-leaves.sh'
LEAF_KEYS = {'peering-outward-key.pem', 'peering-inward-key.pem',
             'republisher-key.pem', 'core-key.pem'}

# Stands in for openssl on PATH: passes every call to the real binary, except
# that the second leaf signature announces itself and then hangs, so the test
# can kill the run at a known point in the middle of minting.
SHIM = '''#!/bin/sh
if [ "$1" = x509 ] && [ "$2" = -req ]; then
  if [ -e "$MINT_SHIM_DIR/signed-once" ]; then
    : > "$MINT_SHIM_DIR/hanging"
    exec sleep 600
  fi
  : > "$MINT_SHIM_DIR/signed-once"
fi
exec "$MINT_REAL_OPENSSL" "$@"
'''


def shape(root):
    """Every path under root, with its type and mode: what 'same shape' means."""
    result = {}
    for path in sorted(root.rglob('*')):
        kind = 'dir' if path.is_dir() and not path.is_symlink() else 'file'
        result[str(path.relative_to(root))] = (kind, path.lstat().st_mode & 0o777)
    return result


def stray_keys(root):
    """Private-key files anywhere under root other than the installed leaves."""
    found = []
    for path in root.rglob('*'):
        if not path.is_file():
            continue
        top_level_leaf = path.parent == root and path.name in LEAF_KEYS
        if top_level_leaf:
            continue
        if path.name.endswith('-key.pem') or b'PRIVATE KEY' in path.read_bytes():
            found.append(str(path.relative_to(root)))
    return sorted(found)


@unittest.skipUnless(shutil.which('openssl'), 'OpenSSL is required')
class InterruptedMint(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        self.ca, self.ca_key = self.root / 'ca.pem', self.root / 'ca-key.pem'
        subprocess.run(['openssl', 'req', '-x509', '-newkey', 'ec',
                        '-pkeyopt', 'ec_paramgen_curve:prime256v1', '-nodes',
                        '-keyout', str(self.ca_key), '-out', str(self.ca), '-days', '365',
                        '-subj', '/CN=Test-CA', '-addext', 'basicConstraints=critical,CA:TRUE'],
                       capture_output=True, check=True)
        der = subprocess.run(['openssl', 'x509', '-in', str(self.ca), '-outform', 'DER'],
                             capture_output=True, check=True).stdout
        self.agreement = self.root / 'agreement.json'
        self.agreement.write_text(json.dumps({
            'parties': [{'sovereign_id': 'xaa',
                         'ca_fingerprint': 'sha256:' + hashlib.sha256(der).hexdigest()},
                        {'sovereign_id': 'xab'}],
            'transport': {'link_subnet': '169.254.10.0/31'},
        }))
        self.env = {k: v for k, v in os.environ.items() if not k.startswith('FED_')}

    def args(self, out, with_key=True):
        return (['bash', str(SCRIPT), str(self.agreement), str(out), '--as', 'xaa',
                 '--ca-cert', str(self.ca), '--core-cn', 'router',
                 '--core-san', 'core.example.test']
                + (['--ca-key', str(self.ca_key)] if with_key else []))

    def mint(self, out):
        result = subprocess.run(self.args(out), env=self.env, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return result

    def clean_shape(self):
        clean = self.root / 'clean'
        self.mint(clean)
        return shape(clean)

    def kill_mid_mint(self, out):
        """Run the mint and SIGKILL its whole process group while it is minting."""
        shim_dir = self.root / 'shim'
        shim_dir.mkdir()
        shim = shim_dir / 'openssl'
        shim.write_text(SHIM)
        shim.chmod(0o755)
        env = dict(self.env, PATH=str(shim_dir) + os.pathsep + self.env.get('PATH', ''),
                   MINT_SHIM_DIR=str(shim_dir), MINT_REAL_OPENSSL=shutil.which('openssl'))
        run = subprocess.Popen(self.args(out), env=env, stdout=subprocess.DEVNULL,
                               stderr=subprocess.DEVNULL, start_new_session=True)
        try:
            deadline = time.monotonic() + 60
            while not (shim_dir / 'hanging').exists():
                self.assertIsNone(run.poll(), 'the mint finished before it could be interrupted')
                self.assertLess(time.monotonic(), deadline, 'the mint never reached its second leaf')
                time.sleep(0.05)
        finally:
            os.killpg(run.pid, signal.SIGKILL)
            run.wait()

    def test_kill_during_mint_then_resume_gives_the_clean_shape(self):
        out = self.root / 'pki'
        self.kill_mid_mint(out)
        # The interruption itself is real: key material was left in staging.
        staged = list(out.glob('.mint-*'))
        self.assertEqual(len(staged), 1, shape(out))
        self.assertTrue(list(staged[0].glob('*-key.pem')))
        self.mint(out)
        self.assertEqual(shape(out), self.clean_shape())
        self.assertEqual(stray_keys(out), [])

    def test_stale_staging_with_keys_is_removed_on_resume(self):
        # The state the lab found: a .mint-* directory holding a full set of
        # staging files, keys among them, and nothing installed yet.
        out = self.root / 'pki'
        out.mkdir(mode=0o700)
        stale = out / '.mint-abandoned'
        stale.mkdir(mode=0o700)
        donor = self.root / 'donor'
        self.mint(donor)
        for name in ('peering-outward', 'peering-inward', 'republisher', 'core'):
            for suffix in ('.pem', '-key.pem'):
                shutil.copy2(donor / (name + suffix), stale / (name + suffix))
        (stale / 'core.csr').write_text('-')
        (stale / 'core.ext').write_text('-')
        self.mint(out)
        self.assertEqual(shape(out), self.clean_shape())
        self.assertEqual(stray_keys(out), [])

    def test_stale_staging_is_removed_when_every_leaf_is_reused(self):
        # A run killed during a rotation's or a missing leaf's mint, after the
        # other leaves were in place: the resumed run issues nothing, and the
        # staging directory must still go.
        out = self.root / 'pki'
        self.mint(out)
        stale = out / '.mint-abandoned'
        stale.mkdir(mode=0o700)
        shutil.copy2(out / 'core-key.pem', stale / 'core-key.pem')
        result = subprocess.run(self.args(out, with_key=False), env=self.env,
                                capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn('reused', result.stdout)
        self.assertEqual(shape(out), self.clean_shape())
        self.assertEqual(stray_keys(out), [])

    def test_a_symlink_named_like_staging_is_removed_without_following_it(self):
        # The sweep unlinks the name; it never deletes through it.
        out = self.root / 'pki'
        out.mkdir(mode=0o700)
        elsewhere = self.root / 'elsewhere'
        elsewhere.mkdir()
        kept = elsewhere / 'live-key.pem'
        kept.write_text('not ours to delete')
        (out / '.mint-link').symlink_to(elsewhere, target_is_directory=True)
        self.mint(out)
        self.assertEqual(kept.read_text(), 'not ours to delete')
        self.assertEqual(shape(out), self.clean_shape())

    def test_a_sweep_never_touches_installed_or_retained_keys(self):
        # Installed leaves and a rotation's retained .previous-* set are live
        # material, not staging: they survive a resumed run unchanged.
        out = self.root / 'pki'
        self.mint(out)
        self.mint_rotate(out)
        previous = list(out.glob('.previous-*'))
        self.assertEqual(len(previous), 1)
        retained = {p.name: p.read_bytes() for p in previous[0].iterdir()}
        installed = {n: (out / n).read_bytes() for n in LEAF_KEYS}
        (out / '.mint-abandoned').mkdir(mode=0o700)
        self.mint(out)
        self.assertEqual({p.name: p.read_bytes() for p in previous[0].iterdir()}, retained)
        self.assertEqual({n: (out / n).read_bytes() for n in LEAF_KEYS}, installed)
        self.assertFalse(list(out.glob('.mint-*')))

    def mint_rotate(self, out):
        result = subprocess.run(self.args(out) + ['--rotate'], env=self.env,
                                capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


if __name__ == '__main__':
    sys.exit(unittest.main())
