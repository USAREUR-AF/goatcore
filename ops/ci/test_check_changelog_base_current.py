#!/usr/bin/env python3
"""ops/ci/check-changelog-base-current.py on a git history of its own.

A branch cut before a goatbuilder release and merged after it carries its entry
rendered in the old Unreleased section; a three-way merge then puts those lines
inside the release and leaves the entry file behind (14 Nov, after 0.7.0's fold).
So a pull request that touches goatbuilder's changelog is refused while its base
has a release its own changelog lacks: it is rebased first, and `changelog.py
write` renders its entry under the new Unreleased."""
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
CHECK = HERE / "check-changelog-base-current.py"
CL = "crates/goatbuilder/CHANGELOG.md"
ENTRY = "crates/goatbuilder/changelog.d/an-entry.md"

UNRELEASED = ("# Changelog\n\n## Unreleased\n\n<!-- BEGIN generated -->\n{body}<!-- END generated -->\n\n"
              "{released}## 1.0.0 — 2099-01-01\n\n### For someone deploying\n\nNothing.\n")
RELEASE_2 = "## 2.0.0 — 2099-02-01\n\n### For someone deploying\n\n#### Fixed: a thing\n\nIt was fixed.\n\n"


def git(repo, *args):
    return subprocess.run(["git", "-C", str(repo), "-c", "user.name=t", "-c", "user.email=t@example.org",
                           "-c", "commit.gpgsign=false", *args], check=True, capture_output=True, text=True).stdout.strip()


def write(repo, path, text):
    p = Path(repo) / path
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(text, encoding="utf-8")


class BaseCurrent(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.r = Path(self._tmp.name) / "repo"
        git(self._tmp.name, "init", "-q", "-b", "main", str(self.r))
        write(self.r, CL, UNRELEASED.format(body="", released=""))
        write(self.r, "README.md", "a repository\n")
        git(self.r, "add", "-A")
        git(self.r, "commit", "-q", "-m", "chore(release): goatbuilder 1.0.0")
        self.cut = git(self.r, "rev-parse", "HEAD")
        # main then releases 2.0.0.
        write(self.r, CL, UNRELEASED.format(body="", released=RELEASE_2))
        git(self.r, "commit", "-q", "-am", "chore(release): goatbuilder 2.0.0")

    def tearDown(self):
        self._tmp.cleanup()

    def branch(self, name, start, files):
        git(self.r, "checkout", "-q", "-b", name, start)
        for path, text in files.items():
            write(self.r, path, text)
        git(self.r, "add", "-A")
        git(self.r, "commit", "-q", "-m", f"a change on {name}")
        head = git(self.r, "rev-parse", "HEAD")
        git(self.r, "checkout", "-q", "main")
        return head

    def run_check(self, head, *extra):
        return subprocess.run([sys.executable, str(CHECK), "--repo", str(self.r), "--base", "main", "--head", head, *extra],
                              capture_output=True, text=True)

    def test_a_branch_cut_before_a_release_that_touches_the_changelog_is_refused(self):
        head = self.branch("stale", self.cut, {ENTRY: "---\nheading: For someone deploying\ntitle: Added: x\n---\n\nx\n",
                                               CL: UNRELEASED.format(body="\n#### Added: x\n\nx\n\n", released="")})
        r = self.run_check(head)
        self.assertEqual(r.returncode, 1, r.stdout + r.stderr)
        self.assertIn("2.0.0", r.stderr)
        self.assertIn("rebase", r.stderr)
        self.assertIn("changelog.py write", r.stderr)

    def test_a_new_entry_file_alone_on_a_stale_branch_is_refused_too(self):
        head = self.branch("stale-entry", self.cut, {ENTRY: "---\nheading: For someone deploying\ntitle: Added: x\n---\n\nx\n"})
        self.assertEqual(self.run_check(head).returncode, 1)

    def test_a_stale_branch_that_does_not_touch_the_changelog_passes(self):
        head = self.branch("stale-other", self.cut, {"README.md": "a repository, edited\n"})
        r = self.run_check(head)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn("does not touch", r.stdout)

    def test_a_branch_on_the_current_base_that_touches_the_changelog_passes(self):
        head = self.branch("current", "main", {ENTRY: "---\nheading: For someone deploying\ntitle: Added: x\n---\n\nx\n"})
        r = self.run_check(head)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)

    def test_without_a_merge_base_it_fails_and_says_why_never_a_silent_pass(self):
        orphan_repo = Path(self._tmp.name) / "other"
        git(self._tmp.name, "init", "-q", "-b", "main", str(orphan_repo))
        write(orphan_repo, CL, "x\n")
        git(orphan_repo, "add", "-A")
        git(orphan_repo, "commit", "-q", "-m", "unrelated")
        git(self.r, "fetch", "-q", str(orphan_repo), "main:unrelated")
        r = self.run_check("unrelated")
        self.assertEqual(r.returncode, 2, r.stdout + r.stderr)
        self.assertIn("merge base", r.stderr)


if __name__ == "__main__":
    unittest.main()
