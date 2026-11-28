#!/usr/bin/env python3
"""The Ory overlay pull keeps a digest-pinned image already on the host.

`docker compose pull` failed the whole Ory redeploy when the registry refused
one image, even when that exact digest was already on the host. The task now
pulls each image, accepts a failed pull only for a digest-pinned image that is
present, and still fails for a tag or for a digest the host lacks. The task's
own script runs here against a stub docker: no daemon, no network."""
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parents[1]
TASKS = ROOT / "ops/ansible/roles/ory/tasks/main.yml"
NAME = "docker compose pull (Ory overlay images)"

# compose config --images prints $IMAGES; a pull succeeds for a name holding
# "reachable"; an image is on the host when its name holds "onhost".
STUB = """#!/bin/bash
case "$1 $2" in
  "compose -f") printf '%s\\n' $IMAGES ;;
  "pull --quiet") case "$3" in *reachable*) exit 0 ;; *) echo "denied: $3" >&2; exit 1 ;; esac ;;
  "image inspect") case "$3" in *onhost*) exit 0 ;; *) exit 1 ;; esac ;;
  *) exit 2 ;;
esac
"""


def task():
    return next(t for t in yaml.safe_load(TASKS.read_text()) if isinstance(t, dict) and t.get("name") == NAME)


class OryOverlayPull(unittest.TestCase):

    def run_script(self, images):
        if not shutil.which("bash"):
            self.skipTest("bash is required")
        d = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, d)
        (d / "bin").mkdir()
        (d / "bin" / "docker").write_text(STUB)
        (d / "bin" / "docker").chmod(0o755)
        (d / "pull.sh").write_text(task()["ansible.builtin.shell"])
        env = dict(os.environ, IMAGES=" ".join(images), PATH=f"{d / 'bin'}:{os.environ['PATH']}")
        return subprocess.run(["bash", str(d / "pull.sh")], cwd=d, env=env, capture_output=True, text=True, timeout=60)

    def test_the_task_runs_under_bash(self):
        self.assertEqual(task()["args"]["executable"], "/bin/bash")

    def test_every_pull_succeeds(self):
        r = self.run_script(["a-reachable:1", "b-reachable@sha256:" + "1" * 64])
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("pulled: a-reachable:1", r.stdout)

    def test_a_present_digest_is_kept_when_its_pull_fails(self):
        r = self.run_script(["a-reachable:1", "b-onhost@sha256:" + "2" * 64])
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("kept, pull failed but the digest is present: b-onhost@sha256:", r.stdout)

    def test_an_absent_digest_fails(self):
        r = self.run_script(["a-reachable:1", "b-gone@sha256:" + "3" * 64])
        self.assertEqual(r.returncode, 1)
        self.assertIn("FAILED: b-gone@sha256:", r.stderr)

    def test_a_tag_fails_even_when_present(self):
        r = self.run_script(["a-reachable:1", "c-onhost:tag"])
        self.assertEqual(r.returncode, 1)
        self.assertIn("FAILED: c-onhost:tag", r.stderr)

    def test_an_overlay_with_no_image_fails(self):
        self.assertEqual(self.run_script([]).returncode, 1)


if __name__ == "__main__":
    unittest.main()
