"""get.<site>/goatbuilder/ serves one reinstall line for a goatbuilder built
before the repository move, and the line installs only what verifies.

A goatbuilder built before releases moved expects the previous release
repository's signature, so it refuses every later release and cannot update
itself (GB-074). The goat-get role's goatbuilder page carries the one line that
reinstalls it. This test renders that page from the role's template, takes the
line, and runs it under bash, sh and zsh with stand-in curl, cosign, uname and
goatbuilder and the real shell, checksum and install tools. No network and no
privilege: downloads come from a local directory, and the only binary the line
installs and runs is a fixture script.

A case passes when the line replaces the installed goatbuilder only after the
signature check (cosign, against the release repository the page names) and the
checksum check both pass, changes nothing when either fails, and leaves no
temporary directory behind.
"""
import hashlib
import html
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import unittest

from jinja2 import Environment, StrictUndefined

ROOT = Path(__file__).resolve().parents[1]
ROLE = ROOT / 'ops/ansible/roles/goat-get'
CATALOG = ROOT / 'ops/errors/catalog.yaml'
HOST = 'get.zagros.example.org'
REPO = 'example-org/goat-release'
TAG = 'goatbuilder/v0.7.3'
PLATFORM = 'linux-amd64'
ASSET = 'goatbuilder-' + PLATFORM

OLD = '#!/bin/sh\necho "goatbuilder 0.4.2 (fixture)"\n'
NEW = '#!/bin/sh\necho "goatbuilder 0.7.3 (fixture)"\n'

# Tools the line may use, linked from the host into a private PATH, so a case
# can leave one out (sha256sum, as on macOS).
REAL_TOOLS = ('mktemp', 'tr', 'sed', 'grep', 'install', 'mkdir', 'dirname', 'rm',
              'cat', 'cp', 'sha256sum', 'shasum', 'perl', 'env')

CURL = r'''#!/bin/sh
# Stand-in curl: copies each URL's basename from $SERVER into the current
# directory (as --remote-name-all would); fails like curl -f on a missing file.
printf 'curl %s\n' "$*" >> "$EVENTS"
for a in "$@"; do
  case "$a" in
    https://*)
      f="${a##*/}"
      [ -f "$SERVER/$f" ] || exit 22
      cp "$SERVER/$f" "./$f" ;;
  esac
done
'''

COSIGN = r'''#!/bin/sh
# Stand-in cosign: records its arguments; verifies only when $COSIGN_OK is 1.
printf 'cosign %s\n' "$*" >> "$EVENTS"
[ "$COSIGN_OK" = 1 ]
'''

UNAME = r'''#!/bin/sh
case "$1" in -s) echo Linux ;; -m) echo x86_64 ;; esac
'''


def render_page():
    source = (ROLE / 'templates/goatbuilder-index.html.j2').read_text()
    return Environment(undefined=StrictUndefined).from_string(source).render(
        goat_get_hostname=HOST,
        goat_get_goatbuilder_release_tag=TAG,
        goat_get_goatbuilder_www_subdir='goatbuilder',
        goat_get_goatbuilder_release_repo=REPO,
        goat_get_goatbuilder_assets=['SHA256SUMS', 'SHA256SUMS.cosign-bundle', ASSET],
        site_id='zagros', site_dns_zone='example.org')


def reinstall_line(page):
    m = re.search(r'<pre id="goatbuilder-reinstall">(.*?)</pre>', page, re.S)
    if m is None:
        raise AssertionError('the goatbuilder page serves no <pre id="goatbuilder-reinstall"> line')
    line = html.unescape(m.group(1))
    if '\n' in line.strip():
        raise AssertionError(f'the reinstall line is more than one line:\n{line}')
    return line.strip()


class ReinstallLine(unittest.TestCase):

    @classmethod
    def setUpClass(cls):
        cls.page = render_page()

    def run_line(self, shell, *, cosign_ok=True, served=NEW, listed=NEW,
                 installed=True, sha256sum=True):
        if not shutil.which(shell):
            self.skipTest(f'{shell} is not installed')
        line = reinstall_line(self.page)
        with tempfile.TemporaryDirectory() as d:
            d = Path(d)
            server, home, tmp, bindir, events = d / 'server', d / 'home', d / 'tmp', d / 'bin', d / 'events'
            for p in (server, home, tmp, bindir):
                p.mkdir()
            events.write_text('')
            (server / ASSET).write_text(served)
            digest = hashlib.sha256(listed.encode()).hexdigest() if listed is not None else None
            sums = f'{"0" * 64}  goatbuilder-darwin-arm64\n'
            if digest:
                sums += f'{digest}  {ASSET}\n'
            (server / 'SHA256SUMS').write_text(sums)
            (server / 'SHA256SUMS.cosign-bundle').write_text('{"fixture": "bundle"}\n')
            for name, text in (('curl', CURL), ('cosign', COSIGN), ('uname', UNAME)):
                (bindir / name).write_text(text)
                (bindir / name).chmod(0o755)
            for tool in REAL_TOOLS:
                if tool == 'sha256sum' and not sha256sum:
                    continue
                real = shutil.which(tool)
                if real is None:
                    if tool == 'shasum' and not sha256sum:
                        self.skipTest('shasum is not installed')
                    continue
                (bindir / tool).symlink_to(real)
            target = home / 'opt-bin' / 'goatbuilder'
            path = str(bindir)
            if installed:
                target.parent.mkdir()
                target.write_text(OLD)
                target.chmod(0o755)
                path = f'{target.parent}:{bindir}'
            env = {'PATH': path, 'HOME': str(home), 'TMPDIR': str(tmp), 'SERVER': str(server),
                   'EVENTS': str(events), 'COSIGN_OK': '1' if cosign_ok else '0'}
            r = subprocess.run([shutil.which(shell), '-c', line], env=env, cwd=home,
                               capture_output=True, text=True, timeout=60)
            installed_at = target if installed else home / '.local/bin/goatbuilder'
            after = installed_at.read_text() if installed_at.exists() else None
            return r, after, events.read_text(), sorted(p.name for p in tmp.iterdir())

    def assert_unchanged(self, r, after, installed=True):
        self.assertNotEqual(r.returncode, 0, f'the line succeeded:\n{r.stdout}{r.stderr}')
        self.assertEqual(after, OLD if installed else None, 'the installed goatbuilder changed')
        self.assertNotIn('0.7.3', r.stdout)

    def test_the_page_serves_one_line(self):
        line = reinstall_line(self.page)
        self.assertIn(f'https://{HOST}/goatbuilder', line)
        self.assertIn(f"--certificate-identity 'https://github.com/{REPO}/.github/workflows/"
                      f"_release-binary.yml@refs/tags/{TAG}'", line)
        self.assertIn('--certificate-oidc-issuer https://token.actions.githubusercontent.com', line)
        # The exact identity alone pins repository, workflow and tag; nothing reads
        # Fulcio's deprecated GitHub workflow extensions.
        self.assertNotIn('--certificate-github-workflow', line)
        self.assertNotIn('--certificate-identity-regexp', line)

    def test_verified_release_replaces_the_installed_binary(self):
        for shell in ('bash', 'sh', 'zsh'):
            with self.subTest(shell):
                r, after, events, left = self.run_line(shell)
                self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
                self.assertEqual(after, NEW)
                self.assertIn('goatbuilder 0.7.3 (fixture)', r.stdout)
                self.assertIn(f"--certificate-identity https://github.com/{REPO}/.github/workflows/"
                              f"_release-binary.yml@refs/tags/{TAG}", events)
                self.assertEqual(left, [], 'the temporary directory was left behind')

    def test_without_sha256sum_it_uses_shasum(self):
        r, after, _, left = self.run_line('bash', sha256sum=False)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual(after, NEW)
        self.assertEqual(left, [])

    def test_no_goatbuilder_on_path_installs_under_local_bin(self):
        r, after, _, _ = self.run_line('bash', installed=False)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual(after, NEW)

    def test_a_signature_that_does_not_verify_changes_nothing(self):
        for shell in ('bash', 'sh', 'zsh'):
            with self.subTest(shell):
                r, after, _, left = self.run_line(shell, cosign_ok=False)
                self.assert_unchanged(r, after)
                self.assertEqual(left, [])

    def test_a_binary_that_does_not_match_sha256sums_changes_nothing(self):
        for sha in (True, False):
            with self.subTest(sha256sum=sha):
                r, after, _, left = self.run_line('bash', served=NEW + '# altered\n', sha256sum=sha)
                self.assert_unchanged(r, after)
                self.assertEqual(left, [])

    def test_a_binary_sha256sums_does_not_list_changes_nothing(self):
        r, after, _, _ = self.run_line('bash', listed=None)
        self.assert_unchanged(r, after)


class CatalogNamesTheStrandedCase(unittest.TestCase):
    """GB-074, the code a binary built before the move refuses with, names that
    case and points at the page's line."""

    def test_gb074(self):
        import yaml
        entries = {e['code']: e for e in yaml.safe_load(CATALOG.read_text())['entries']}
        e = entries['GB-074']
        causes = ' '.join(e['causes'])
        fixes = ' '.join(e['fixes'])
        self.assertIn('before the repository move', causes)
        self.assertIn('previous release repository', causes)
        self.assertIn('/goatbuilder/', fixes)
        self.assertIn('Built before the repository move', fixes)
        self.assertIn('cosign verify-blob', fixes)


if __name__ == '__main__':
    unittest.main()
