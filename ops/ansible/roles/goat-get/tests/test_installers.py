#!/usr/bin/env python3
"""Execute the rendered Unix install commands without network or privileges.

Requires Jinja2, bash, sh, sha256sum and shasum. Downloads, sudo, codesign
and goat are fixtures; the shell, mktemp and checksum tools are real.
Run from any directory: python3 ops/ansible/roles/goat-get/tests/test_installers.py
"""

import hashlib
from html.parser import HTMLParser
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import unittest

from jinja2 import Environment, StrictUndefined


REPO = Path(os.environ.get("GOAT_INSTALL_REPO", Path(__file__).resolve().parents[5]))
PLATFORMS = ("linux-amd64", "linux-arm64", "darwin-amd64", "darwin-arm64")
ASSETS = ["goat-" + p for p in (*PLATFORMS, "windows-amd64.exe")]
TRUSTED = b"TRUSTED_FIXTURE_BINARY\n"
UNTRUSTED = b"UNTRUSTED_FIXTURE_BINARY\n"


def checksum(data):
    return hashlib.sha256(data).hexdigest()


class Panels(HTMLParser):
    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.platform = None
        self.in_pre = False
        self.blocks = {}

    def handle_starttag(self, tag, attrs):
        if tag == "section":
            self.platform = dict(attrs).get("data-plat")
        if tag == "pre" and self.platform:
            self.in_pre = True
            self.blocks[self.platform] = ""

    def handle_endtag(self, tag):
        if tag == "pre":
            self.in_pre = False
        if tag == "section":
            self.platform = None

    def handle_data(self, text):
        if self.in_pre:
            self.blocks[self.platform] += text


# No fixture ever calls real sudo or executes downloaded bytes. Limit the
# privileged-command stub to a redirected, private install destination.
MOCK = '''import os, pathlib, shutil, subprocess, sys
tool = pathlib.Path(sys.argv[0]).name
args = sys.argv[1:]
case = os.environ['CASE']
with open(os.environ['EVENTS'], 'a') as f:
    f.write(tool + ':' + ' '.join(args) + '\\n')
if tool == 'uname':
    print(os.environ['FAKE_OS'] if args == ['-s'] else os.environ['FAKE_ARCH'])
elif tool == 'curl':
    asset = args[1].rsplit('/', 1)[-1]
    dest = pathlib.Path(args[args.index('-o') + 1])
    if case == 'both_fetches_failed_with_cache':
        sys.exit(22)
    if case == 'binary_fetch_failed_with_cache' and not asset.endswith('.sha256'):
        sys.exit(22)
    if case == 'checksum_fetch_failed_with_cache' and asset.endswith('.sha256'):
        sys.exit(22)
    if case == 'partial_binary_fetch' and not asset.endswith('.sha256'):
        dest.write_bytes(b'PARTIAL_BYTES')
        sys.exit(18)
    source = pathlib.Path(os.environ['SERVER']) / asset
    if not source.exists():
        sys.exit(22)
    shutil.copyfile(source, dest)
elif tool == 'sudo':
    dest = pathlib.Path(os.environ['DEST'])
    if args[0] == 'mkdir':
        assert args == ['mkdir', '-p', str(dest)]
        if case == 'mkdir_failed':
            sys.exit(1)
    else:
        assert args[0:3] == ['install', '-m', '0755']
        assert pathlib.Path(args[-1]) == dest / 'goat'
        assert pathlib.Path(args[-2]).name.startswith('goat-')
        if case == 'install_failed':
            sys.exit(1)
    sys.exit(subprocess.call(args))
elif tool == 'codesign':
    if case == 'codesign_failed':
        sys.exit(1)
    with pathlib.Path(args[-1]).open('ab') as binary:
        binary.write(b'RESIGNED_FIXTURE\\n')
elif tool == 'goat':
    assert args == ['--version']
    print('goat fixture-version')
else:
    raise SystemExit('unexpected mocked tool')
'''


class Installers(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        for tool in ("bash", "sh", "mktemp", "sha256sum", "shasum"):
            if not shutil.which(tool):
                raise RuntimeError("required test tool missing: " + tool)
        source = (REPO / "ops/ansible/roles/goat-get/templates/index.html.j2").read_text()
        cls.rendered = Environment(undefined=StrictUndefined).from_string(source).render(
            goat_get_hostname="download.example.invalid",
            goat_get_release_tag="fixture-v0",
            goat_get_assets=ASSETS + [a + ".sha256" for a in ASSETS] + ["SHA256SUMS", "SHA256SUMS.cosign-bundle"],
        )
        panels = Panels()
        panels.feed(cls.rendered)
        cls.panels = panels.blocks
        cls.surfaces = [("page-" + p, p, cls.panels[p]) for p in PLATFORMS]
        cls.docs = (REPO / "docs/operations/goat-cli-install.md").read_text()
        blocks = [b for b in re.findall(r"```bash\n(.*?)```", cls.docs, re.S)
                  if 'BIN="goat-${OS}-${ARCH}"' in b]
        if len(blocks) != 2:
            raise AssertionError("expected mesh-install and old-version fallback blocks")
        for name, block in zip(("doc-mesh", "doc-fallback"), blocks):
            for p in ("linux-amd64", "darwin-arm64"):
                cls.surfaces.append((name + "-" + p, p, block))

    def test_windows_and_profile_contracts(self):
        self.assertEqual(self.panels["windows-amd64"],
                         "curl -fsSL https://download.example.invalid/goat-windows-amd64.exe "
                         "-o %USERPROFILE%\\goat.exe\n%USERPROFILE%\\goat.exe --version")
        self.assertLess(self.rendered.index("goat ca add"),
                        self.rendered.index("goat profile init"))
        self.assertIn('sudo /usr/local/bin/goat update --profile-dir "$HOME/.goat"', self.docs)

    def test_download_and_install_failures(self):
        cases = ("valid", "tampered", "both_fetches_failed_with_cache", "binary_fetch_failed_with_cache",
                 "checksum_fetch_failed_with_cache", "partial_binary_fetch",
                 "missing_selected_entry", "malformed_checksum", "duplicate_checksum",
                 "stale_other_arch", "codesign_failed", "mkdir_failed", "install_failed")
        for shell in ("bash", "sh"):
            for name, platform, script in self.surfaces:
                for case in cases:
                    if case == "codesign_failed" and not platform.startswith("darwin"):
                        continue
                    with self.subTest(shell=shell, surface=name, case=case):
                        self.check_install(shell, platform, script, case)

    def check_install(self, shell, platform, original, case):
        # Spaces exercise quoting of mktemp's output and destination paths.
        with tempfile.TemporaryDirectory(prefix="goat install test ") as scratch:
            root = Path(scratch)
            stage, server, mocks, dest = [root / p for p in ("tmp", "server", "mocks", "dest")]
            for directory in (stage, server, mocks, dest):
                directory.mkdir()
            selected = "goat-" + platform
            other = "goat-linux-arm64" if selected != "goat-linux-arm64" else "goat-linux-amd64"
            (server / selected).write_bytes(UNTRUSTED if case == "tampered" else TRUSTED)
            sidecar = checksum(TRUSTED) + "  " + selected + "\n"
            if case == "missing_selected_entry":
                sidecar = checksum(TRUSTED) + "  " + other + "\n"
            elif case == "malformed_checksum":
                sidecar = "not-a-sha256  " + selected + "\n"
            elif case == "duplicate_checksum":
                sidecar += sidecar
            (server / (selected + ".sha256")).write_text(sidecar)
            (server / "SHA256SUMS").write_text(sidecar)
            cached = {selected: UNTRUSTED, selected + ".sha256":
                      (checksum(UNTRUSTED) + "  " + selected + "\n").encode(),
                      "SHA256SUMS": (checksum(UNTRUSTED) + "  " + selected + "\n").encode(),
                      other: b"OLDER_RELEASE_BYTES\n"}
            for asset, data in cached.items():
                (stage / asset).write_bytes(data)
            if case == "stale_other_arch":
                with (server / "SHA256SUMS").open("a") as manifest:
                    manifest.write(checksum(TRUSTED) + "  " + other + "\n")
            for tool in ("uname", "curl", "sudo", "codesign", "goat"):
                path = mocks / tool
                path.write_text("#!" + sys.executable + "\n" + MOCK)
                path.chmod(0o755)
            # Redirect only absolute install/shared-cache paths. Keep the
            # downloaded filename, control flow and checksum code unchanged.
            script = re.sub(r'https://[^/"\s]+/', 'https://download.example.invalid/', original)
            script = script.replace("/usr/local/bin/goat --version", '"__GOAT_MOCK__" --version')
            script = script.replace("/usr/local/bin/goat", '"__GOAT_DEST__/goat"')
            script = script.replace("/usr/local/bin", '"__GOAT_DEST__"')
            script = re.sub(r'(?<!["\w])/tmp/([^\s"]+)',
                            lambda match: '"__GOAT_CACHE__/' + match[1] + '"', script)
            script = script.replace("/tmp/", "__GOAT_CACHE__/").replace("cd /tmp", 'cd "__GOAT_CACHE__"')
            # Expand placeholders last: on Linux, injected fixture paths also
            # start with /tmp and must never be redirected a second time.
            script = script.replace("__GOAT_CACHE__", str(stage))
            script = script.replace("__GOAT_DEST__", str(dest))
            script = script.replace("__GOAT_MOCK__", str(mocks / "goat"))
            env = dict(os.environ, PATH=str(mocks) + os.pathsep + os.environ["PATH"],
                       TMPDIR=str(stage), CASE=case, SERVER=str(server), DEST=str(dest),
                       EVENTS=str(root / "events"),
                       FAKE_OS="Darwin" if platform.startswith("darwin") else "Linux",
                       FAKE_ARCH="arm64" if platform.endswith("arm64") else "x86_64")
            result = subprocess.run([shell, "-c", script], cwd=REPO, env=env, text=True,
                                    stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=15)
            output = result.stdout.replace(str(root), "[fixture]")
            events = (root / "events").read_text().replace(str(root), "[fixture]")
            expected = case in ("valid", "stale_other_arch")
            installed = dest / "goat"
            self.assertEqual(installed.exists(), expected, output + events)
            self.assertEqual(result.returncode == 0, expected, output + events)
            if expected:
                expected_bytes = TRUSTED + (b"RESIGNED_FIXTURE\n" if platform.startswith("darwin") else b"")
                self.assertEqual(installed.read_bytes(), expected_bytes)
                self.assertEqual(installed.stat().st_mode & 0o777, 0o755)
                if platform.startswith("darwin"):
                    self.assertEqual(events.count("codesign:"), 1)
                    self.assertLess(events.index("codesign:"), events.index("sudo:install"))
                else:
                    self.assertNotIn("codesign:", events)
            if case not in ("valid", "stale_other_arch", "install_failed"):
                self.assertNotIn("sudo:install", events)
            if not expected:
                self.assertNotIn("goat:--version", events)
            # Fresh download staging is removed on success and on every error;
            # pre-existing shared cache files are never overwritten or removed.
            self.assertEqual({p.name for p in stage.iterdir()}, set(cached))
            for asset, data in cached.items():
                self.assertEqual((stage / asset).read_bytes(), data)


if __name__ == "__main__":
    unittest.main(verbosity=2)
