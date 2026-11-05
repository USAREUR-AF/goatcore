#!/usr/bin/env python3
"""fresh-source.sh judges a checkout against GOAT_ANSIBLE_TRUNK_BRANCH of the trunk remote.

A fork whose deploy trunk is a branch other than `main` of its mirror (the branch it actually
deploys) sets GOAT_ANSIBLE_TRUNK_BRANCH; unset keeps `main`. Builds throwaway git repos; no
network, no host.

Run: python3 -m unittest tests/test_fresh_source_trunk_branch.py -v
"""
import os
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
FRESH = ROOT / "ops/scripts/fresh-source.sh"


def git(cwd, *args):
    env = {**os.environ, "GIT_AUTHOR_NAME": "t", "GIT_AUTHOR_EMAIL": "t@example.invalid",
           "GIT_COMMITTER_NAME": "t", "GIT_COMMITTER_EMAIL": "t@example.invalid"}
    return subprocess.run(["git", "-C", str(cwd), *args], check=True, capture_output=True,
                          text=True, env=env).stdout.strip()


class TrunkBranch(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        t = Path(self.tmp.name)
        src = t / "src"
        src.mkdir()
        git(src, "init", "-q", "-b", "main")
        (src / "ops/ansible").mkdir(parents=True)
        (src / "ops/ansible/play.yml").write_text("a: 1\n")
        git(src, "add", ".")
        git(src, "commit", "-q", "-m", "base")
        self.main_sha = git(src, "rev-parse", "HEAD")
        git(src, "checkout", "-q", "-b", "azure")
        (src / "ops/ansible/play.yml").write_text("a: 2\n")
        git(src, "commit", "-q", "-am", "azure fix")
        self.azure_sha = git(src, "rev-parse", "HEAD")
        self.mirror = t / "mirror.git"
        git(t, "clone", "-q", "--bare", str(src), str(self.mirror))
        self.work = t / "work"
        git(t, "clone", "-q", "-o", "mirror", str(self.mirror), str(self.work))

    def tearDown(self):
        self.tmp.cleanup()

    def run_fresh(self, at, branch=None):
        git(self.work, "checkout", "-q", "--detach", at)
        env = {k: v for k, v in os.environ.items()
               if k not in ("GOAT_ANSIBLE_TRUNK_BRANCH", "GOAT_FRESH_SOURCE_TRUNK", "GOAT_DEPLOY_OFFLINE")}
        if branch:
            env["GOAT_ANSIBLE_TRUNK_BRANCH"] = branch
        return subprocess.run(["bash", str(FRESH), "--repo", str(self.work), "--remote", "mirror"],
                              capture_output=True, text=True, env=env)

    def test_default_trunk_is_main(self):
        r = self.run_fresh(self.main_sha)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual(r.stdout.strip(), self.main_sha)

    def test_trunk_branch_refuses_a_checkout_behind_it(self):
        r = self.run_fresh(self.main_sha, branch="azure")
        self.assertEqual(r.returncode, 1, r.stdout + r.stderr)
        self.assertIn("GB-064", r.stdout + r.stderr)
        self.assertIn("mirror/azure", r.stdout + r.stderr)

    def test_trunk_branch_passes_its_own_head(self):
        r = self.run_fresh(self.azure_sha, branch="azure")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual(r.stdout.strip(), self.azure_sha)

    def test_missing_trunk_branch_refused(self):
        r = self.run_fresh(self.azure_sha, branch="nonesuch")
        self.assertEqual(r.returncode, 1, r.stdout + r.stderr)
        self.assertIn("GB-065", r.stdout + r.stderr)


if __name__ == "__main__":
    unittest.main()
