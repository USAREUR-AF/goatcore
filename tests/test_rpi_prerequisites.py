#!/usr/bin/env python3
"""Focused offline tests for Raspberry Pi Ubuntu image verification."""

import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import textwrap
import unittest


ROOT = Path(__file__).resolve().parents[1]
IMAGE_LIB = ROOT / "ops" / "rpi-base-image" / "lib" / "ubuntu-image.sh"
BASH = shutil.which("bash") or "/bin/bash"
IMAGE_NAME = "ubuntu-24.04.4-preinstalled-server-arm64+raspi.img.xz"
RELEASE_URL = "https://example.invalid/releases/24.04/release"


class UbuntuImageVerification(unittest.TestCase):
    def setUp(self):
        self.temp_context = tempfile.TemporaryDirectory()
        self.tmp = Path(self.temp_context.name)
        self.bin = self.tmp / "bin"
        self.bin.mkdir()
        self.cache = self.tmp / "cache"
        self.fallback = self.tmp / "fallback"
        self.cache.mkdir()
        self.fallback.mkdir()
        self.payload = self.tmp / "remote-image"
        self.payload.write_bytes(b"verified Ubuntu image fixture\n")
        self.digest = hashlib.sha256(self.payload.read_bytes()).hexdigest()
        self.manifest = self.tmp / "SHA256SUMS"
        self.manifest.write_text(f"{self.digest} *{IMAGE_NAME}\n")
        self.log = self.tmp / "curl.log"
        self.write_executable(
            "curl",
            r"""
            #!/usr/bin/env bash
            output=""
            want_output=0
            for arg in "$@"; do
                if [ "$want_output" -eq 1 ]; then
                    output="$arg"
                    want_output=0
                elif [ "$arg" = "--output" ]; then
                    want_output=1
                fi
            done
            url="${!#}"
            printf '%s\n' "$url" >> "$STUB_CURL_LOG"
            if [ "${STUB_FAIL_URL:-}" = "$url" ]; then
                printf 'stubbed download failure: %s\n' "$url" >&2
                [ -n "$output" ] && printf 'partial' > "$output"
                exit 22
            fi
            case "$url" in
                */SHA256SUMS) /bin/cp "$STUB_MANIFEST" "$output" ;;
                */"$STUB_IMAGE_NAME") /bin/cp "$STUB_IMAGE" "$output" ;;
                *) printf 'unexpected URL: %s\n' "$url" >&2; exit 2 ;;
            esac
            """,
        )

    def tearDown(self):
        self.temp_context.cleanup()

    def write_executable(self, name, body):
        path = self.bin / name
        path.write_text(textwrap.dedent(body).lstrip())
        path.chmod(0o755)

    def env(self, **updates):
        env = os.environ.copy()
        env.update(
            PATH=f"{self.bin}{os.pathsep}{env['PATH']}",
            STUB_CURL_LOG=str(self.log),
            STUB_MANIFEST=str(self.manifest),
            STUB_IMAGE=str(self.payload),
            STUB_IMAGE_NAME=IMAGE_NAME,
            TMPDIR=str(self.tmp),
        )
        env.update(updates)
        return env

    def run_prepare(self, **env_updates):
        script = r'''
            set -euo pipefail
            info() { printf 'INFO %s\n' "$*"; }
            ok() { printf 'OK %s\n' "$*"; }
            error() { printf 'ERROR %s\n' "$*" >&2; }
            source "$1"
            prepare_ubuntu_image "$2" "$3" "$4" "$5"
            printf 'NEXT %s\n' "$IMAGE_PATH"
        '''
        return subprocess.run(
            [
                BASH,
                "-c",
                textwrap.dedent(script),
                "_",
                str(IMAGE_LIB),
                str(self.cache),
                str(self.fallback),
                RELEASE_URL,
                IMAGE_NAME,
            ],
            capture_output=True,
            text=True,
            timeout=30,
            env=self.env(**env_updates),
        )

    def test_new_download_is_verified_before_it_enters_the_cache(self):
        result = self.run_prepare()
        final = self.cache / IMAGE_NAME
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(final.read_bytes(), self.payload.read_bytes())
        self.assertIn(f"NEXT {final}", result.stdout)
        self.assertFalse(list(self.cache.glob("*.part.*")))

    def test_exact_cached_image_is_verified_without_image_download(self):
        cached = self.cache / IMAGE_NAME
        cached.write_bytes(self.payload.read_bytes())
        result = self.run_prepare()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(f"NEXT {cached}", result.stdout)
        urls = self.log.read_text().splitlines()
        self.assertEqual(urls, [f"{RELEASE_URL}/SHA256SUMS"])

    def test_exact_image_in_legacy_fallback_cache_is_verified(self):
        cached = self.fallback / IMAGE_NAME
        cached.write_bytes(self.payload.read_bytes())
        result = self.run_prepare()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(f"NEXT {cached}", result.stdout)
        self.assertFalse((self.cache / IMAGE_NAME).exists())

    def test_arbitrary_cached_version_is_not_selected_for_the_pinned_release(self):
        other = self.cache / "ubuntu-24.04.3-preinstalled-server-arm64+raspi.img.xz"
        other.write_bytes(b"older fixture")
        result = self.run_prepare()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(other.read_bytes(), b"older fixture")
        self.assertTrue((self.cache / IMAGE_NAME).is_file())

    def test_cached_mismatch_is_refused_and_preserved(self):
        cached = self.cache / IMAGE_NAME
        cached.write_bytes(b"wrong bytes")
        result = self.run_prepare()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("SHA-256 mismatch", result.stderr)
        self.assertNotIn("NEXT ", result.stdout)
        self.assertEqual(cached.read_bytes(), b"wrong bytes")

    def test_downloaded_mismatch_is_removed_and_never_enters_the_cache(self):
        self.manifest.write_text(f"{'0' * 64} *{IMAGE_NAME}\n")
        result = self.run_prepare()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("SHA-256 mismatch", result.stderr)
        self.assertNotIn("NEXT ", result.stdout)
        self.assertFalse((self.cache / IMAGE_NAME).exists())
        self.assertFalse(list(self.cache.glob("*.part.*")))

    def test_missing_malformed_and_duplicate_exact_entries_are_refused(self):
        cases = {
            "missing": f"{self.digest} *unrelated.img.xz\n",
            "malformed": f"not-a-digest *{IMAGE_NAME}\n",
            "duplicate": (
                f"{self.digest} *{IMAGE_NAME}\n"
                f"{self.digest} *{IMAGE_NAME}\n"
            ),
        }
        for label, content in cases.items():
            with self.subTest(label=label):
                self.manifest.write_text(content)
                result = self.run_prepare()
                self.assertNotEqual(result.returncode, 0)
                self.assertNotIn("NEXT ", result.stdout)
                self.assertFalse((self.cache / IMAGE_NAME).exists())

    def test_unrelated_manifest_paths_are_ignored_and_contents_are_not_executed(self):
        marker = self.tmp / "must-not-exist"
        self.manifest.write_text(
            f"{self.digest} *../../unrelated.img.xz\n"
            f"$(touch {marker}) *another.img.xz\n"
            f"{self.digest} *{IMAGE_NAME}\n"
        )
        result = self.run_prepare()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(marker.exists())

    def test_failed_image_download_removes_partial_and_blocks_next_step(self):
        result = self.run_prepare(STUB_FAIL_URL=f"{RELEASE_URL}/{IMAGE_NAME}")
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("NEXT ", result.stdout)
        self.assertFalse((self.cache / IMAGE_NAME).exists())
        self.assertFalse(list(self.cache.glob("*.part.*")))

    def test_failed_manifest_download_preserves_cached_image_and_blocks_next_step(self):
        cached = self.cache / IMAGE_NAME
        cached.write_bytes(self.payload.read_bytes())
        result = self.run_prepare(STUB_FAIL_URL=f"{RELEASE_URL}/SHA256SUMS")
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("NEXT ", result.stdout)
        self.assertEqual(cached.read_bytes(), self.payload.read_bytes())

    def test_sha256sum_shasum_and_openssl_branches(self):
        tools = ("sha256sum", "shasum", "openssl")
        for tool in tools:
            executable = shutil.which(tool)
            if executable is None:
                self.skipTest(f"{tool} is unavailable")
            with self.subTest(tool=tool), tempfile.TemporaryDirectory() as path_dir:
                path = Path(path_dir)
                for command in (tool, "tr", "grep"):
                    source = executable if command == tool else shutil.which(command)
                    os.symlink(source, path / command)
                script = r'''
                    error() { printf 'ERROR %s\n' "$*" >&2; }
                    source "$1"
                    image_sha256 "$2"
                '''
                result = subprocess.run(
                    [BASH, "-c", textwrap.dedent(script), "_", str(IMAGE_LIB), str(self.payload)],
                    capture_output=True,
                    text=True,
                    timeout=30,
                    env={"PATH": str(path)},
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stdout.strip(), self.digest)


if __name__ == "__main__":
    unittest.main()
