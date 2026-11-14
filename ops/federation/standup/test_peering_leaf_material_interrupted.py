"""An interrupted phase-8 mint leaves no private key behind, and an interrupted
rotation is finished or rolled back by the next run, never minted a third time.

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


# Stands in for python3 on PATH, for the leaf helper only: os.replace, which
# installs each new file of a rotation, ends the process abruptly (os._exit: no
# handler, no cleanup) on its KILL_AT-th call, as a kill -9 or a lost host would.
KILL_BOOT = '''
import importlib.util, os, sys
spec = importlib.util.spec_from_file_location("peering_leaf_material", sys.argv[1])
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
real, calls = os.replace, [0]
def replace(source, target):
    calls[0] += 1
    if calls[0] == int(os.environ["KILL_AT"]):
        os._exit(137)
    return real(source, target)
module.os.replace = replace
sys.argv = sys.argv[1:]
module.main()
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

    def args(self, out, with_key=True, rotate=False):
        return (['bash', str(SCRIPT), str(self.agreement), str(out), '--as', 'xaa',
                 '--ca-cert', str(self.ca), '--core-cn', 'router',
                 '--core-san', 'core.example.test']
                + (['--ca-key', str(self.ca_key)] if with_key else [])
                + (['--rotate'] if rotate else []))

    def mint(self, out):
        result = subprocess.run(self.args(out), env=self.env, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return result

    def clean_shape(self):
        clean = self.root / 'clean'
        self.mint(clean)
        return shape(clean)

    def kill_mid_mint(self, out, rotate=False):
        """Run the mint and SIGKILL its whole process group while it is minting."""
        shim_dir = self.root / 'shim'
        shim_dir.mkdir()
        shim = shim_dir / 'openssl'
        shim.write_text(SHIM)
        shim.chmod(0o755)
        env = dict(self.env, PATH=str(shim_dir) + os.pathsep + self.env.get('PATH', ''),
                   MINT_SHIM_DIR=str(shim_dir), MINT_REAL_OPENSSL=shutil.which('openssl'))
        run = subprocess.Popen(self.args(out, rotate=rotate), env=env, stdout=subprocess.DEVNULL,
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
        self.assertTrue(list(staged[0].rglob('*-key.pem')))
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
        # Installed leaves and a .previous-* set an earlier release retained
        # are not staging: they survive a resumed run unchanged.
        out = self.root / 'pki'
        self.mint(out)
        self.mint_rotate(out)
        self.assertFalse(list(out.glob('.previous-*')), 'a committed rotation keeps no previous set')
        older = out / '.previous-older'
        older.mkdir(mode=0o700)
        shutil.copy2(out / 'core-key.pem', older / 'core-key.pem')
        previous = [older]
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
        return result

    def leaves(self, out):
        """Every installed leaf file, by name, with its bytes."""
        return {p.name: p.read_bytes() for p in out.glob('*.pem') if p.name != 'our-ca.pem'}

    def kill_mid_install(self, out, at=2):
        """Rotate, and end the helper abruptly at its at-th install of a new file."""
        shim_dir = self.root / 'pyshim'
        shim_dir.mkdir(exist_ok=True)
        (shim_dir / 'boot.py').write_text(KILL_BOOT)
        shim = shim_dir / 'python3'
        shim.write_text('#!/bin/sh\ncase "$1" in\n  *peering_leaf_material.py) exec "$REAL_PYTHON3" "$KILL_BOOT_PY" "$@" ;;\n'
                        'esac\nexec "$REAL_PYTHON3" "$@"\n')
        shim.chmod(0o755)
        env = dict(self.env, PATH=str(shim_dir) + os.pathsep + self.env.get('PATH', ''),
                   REAL_PYTHON3=sys.executable, KILL_BOOT_PY=str(shim_dir / 'boot.py'), KILL_AT=str(at))
        result = subprocess.run(self.args(out, rotate=True), env=env, capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0, result.stdout)
        pending = out / '.rotation-pending'
        self.assertTrue(pending.is_dir(), shape(out))
        return pending

    def test_a_rotation_killed_while_installing_is_finished_and_never_mints_a_third_pair(self):
        for rerun_rotates in (True, False):
            with self.subTest(rerun_rotates=rerun_rotates):
                out = self.root / ('pki-' + str(rerun_rotates))
                self.mint(out)
                old = self.leaves(out)
                pending = self.kill_mid_install(out)
                # The set-aside pairs are the owner's alone, under the peering's own folder.
                self.assertEqual(pending.lstat().st_mode & 0o777, 0o700)
                kept = sorted((pending / 'previous').iterdir())
                self.assertEqual({p.name for p in kept}, set(old))
                for path in kept:
                    self.assertEqual(path.lstat().st_mode & 0o777, 0o600, path.name)
                    self.assertEqual(path.read_bytes(), old[path.name])
                # The one new pair minted: what is installed, then what waits.
                minted = self.leaves(out)
                minted.update({p.name: p.read_bytes() for p in (pending / 'new').glob('*.pem')})
                self.assertEqual(set(minted), set(old))
                self.assertTrue(all(minted[n] != old[n] for n in old))
                result = self.mint_rotate(out) if rerun_rotates else self.mint(out)
                self.assertIn('finished', result.stdout)
                self.assertEqual(self.leaves(out), minted, 'the run minted again')
                self.assertFalse(pending.exists())
                self.assertFalse(list(out.glob('.previous-*')))
                self.assertEqual(stray_keys(out), [])
                self.assertEqual(shape(out), self.clean_shape_once())

    def test_a_rotation_that_cannot_be_finished_is_rolled_back_to_the_previous_pairs(self):
        out = self.root / 'pki'
        self.mint(out)
        old = self.leaves(out)
        pending = self.kill_mid_install(out)
        waiting = sorted((pending / 'new').glob('*-key.pem'))
        self.assertTrue(waiting)
        waiting[0].write_text('not a key\n')
        result = subprocess.run(self.args(out), env=self.env, capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn('restored', result.stdout + result.stderr)
        self.assertEqual(self.leaves(out), old)
        for name in old:
            want = 0o600 if name.endswith('-key.pem') else 0o644
            self.assertEqual((out / name).lstat().st_mode & 0o777, want, name)
        self.assertFalse(pending.exists())
        self.assertEqual(stray_keys(out), [])
        # Nothing is left half done: the next rotation starts afresh.
        self.mint_rotate(out)
        self.assertTrue(all(self.leaves(out)[n] != old[n] for n in old))

    def test_a_rotation_killed_while_minting_leaves_the_previous_pairs_in_place(self):
        out = self.root / 'pki'
        self.mint(out)
        old = self.leaves(out)
        self.kill_mid_mint(out, rotate=True)
        self.assertEqual(self.leaves(out), old)
        self.assertFalse((out / '.rotation-pending').exists())
        self.mint_rotate(out)
        self.assertTrue(all(self.leaves(out)[n] != old[n] for n in old))
        self.assertEqual(stray_keys(out), [])
        self.assertEqual(shape(out), self.clean_shape_once())

    def clean_shape_once(self):
        if not hasattr(self, '_clean'):
            self._clean = self.clean_shape()
        return self._clean


if __name__ == '__main__':
    sys.exit(unittest.main())
