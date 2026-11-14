#!/usr/bin/env python3
"""ops/ci/check-golden-base-current.py on a git history of its own.

crates/goatbuilder/tests/full_plans_unchanged.rs holds every full fixture site's standup
plan to a recorded file, full-plans.golden.json. A pull request that records the file on
a base older than a change to what the plans are made from (standup.rs, the fixture sites,
or the record itself) is green on its own base and turns main red once merged: on 14 Nov a
record cut before a phase 12 title change merged after it. So a pull request that changes
the record is refused while main has such a commit its own history lacks; it is rebased
and the record regenerated."""
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
CHECK = HERE / "check-golden-base-current.py"
GOLDEN = "crates/goatbuilder/tests/fixtures/full-plans.golden.json"
STANDUP = "crates/goatbuilder/src/standup.rs"
SITE = "ops/goatbuilder/fixtures/basic/goatbuilder.yaml"


def git(repo, *args):
    return subprocess.run(["git", "-C", str(repo), "-c", "user.name=t", "-c", "user.email=t@example.org",
                           "-c", "commit.gpgsign=false", *args], check=True, capture_output=True, text=True).stdout.strip()


def write(repo, path, text):
    p = Path(repo) / path
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(text, encoding="utf-8")


class GoldenBaseCurrent(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.r = Path(self._tmp.name) / "repo"
        git(self._tmp.name, "init", "-q", "-b", "main", str(self.r))
        for path, text in ((GOLDEN, "[\"phase 12: joiners use the founder's DNS\"]\n"),
                           (STANDUP, "// phase 12: joiners use the founder's DNS\n"),
                           (SITE, "site: basic\n"), ("README.md", "a repository\n")):
            write(self.r, path, text)
        git(self.r, "add", "-A")
        git(self.r, "commit", "-q", "-m", "the record and what it is made from")
        self.cut = git(self.r, "rev-parse", "HEAD")

    def tearDown(self):
        self._tmp.cleanup()

    def main_commit(self, path, text, msg):
        git(self.r, "checkout", "-q", "main")
        write(self.r, path, text)
        git(self.r, "commit", "-q", "-am", msg)
        return git(self.r, "rev-parse", "HEAD")

    def branch(self, name, start, files):
        git(self.r, "checkout", "-q", "-b", name, start)
        for path, text in files.items():
            write(self.r, path, text)
        git(self.r, "add", "-A")
        git(self.r, "commit", "-q", "-m", f"a change on {name}")
        head = git(self.r, "rev-parse", "HEAD")
        git(self.r, "checkout", "-q", "main")
        return head

    def run_check(self, head, base="main"):
        return subprocess.run([sys.executable, str(CHECK), "--repo", str(self.r), "--base", base, "--head", head],
                              capture_output=True, text=True)

    def test_a_record_cut_before_a_standup_change_on_main_is_refused(self):
        head = self.branch("pr", self.cut, {GOLDEN: "[\"a plan recorded on the old base\"]\n"})
        self.main_commit(STANDUP, "// phase 12: every site runs its own resolver\n", "feat: phase 12's title")
        r = self.run_check(head)
        self.assertEqual(r.returncode, 1, r.stdout + r.stderr)
        self.assertIn("feat: phase 12's title", r.stderr, "it names the commit the branch lacks")
        self.assertIn("GOATBUILDER_GOLDEN=overwrite cargo test -p goatbuilder --test full_plans_unchanged",
                      r.stderr, "and the command that regenerates the record")

    def test_a_fixture_site_change_or_another_record_on_main_also_refuses(self):
        for n, path in enumerate((SITE, GOLDEN)):
            head = self.branch(f"pr-{n}", self.cut, {GOLDEN: "[\"recorded on the old base %d\"]\n" % n})
            self.main_commit(path, "changed on main: %s\n" % path, f"change {path} on main")
            r = self.run_check(head)
            self.assertEqual(r.returncode, 1, (path, r.stdout + r.stderr))

    def test_a_record_on_a_current_base_passes_and_says_so(self):
        self.main_commit(STANDUP, "// phase 12: every site runs its own resolver\n", "feat: phase 12's title")
        head = self.branch("pr", "main", {GOLDEN: "[\"recorded on the current base\"]\n"})
        r = self.run_check(head)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn("ok", r.stdout)

    def test_a_pull_request_that_does_not_change_the_record_passes_and_says_so(self):
        head = self.branch("pr", self.cut, {"README.md": "a repository, reworded\n"})
        self.main_commit(STANDUP, "// phase 12: every site runs its own resolver\n", "feat: phase 12's title")
        r = self.run_check(head)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn("does not change", r.stdout)

    def test_an_unrelated_commit_on_main_does_not_refuse(self):
        head = self.branch("pr", self.cut, {GOLDEN: "[\"recorded on the old base\"]\n"})
        self.main_commit("README.md", "a repository, elsewhere changed\n", "docs: readme")
        r = self.run_check(head)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)

    def test_no_merge_base_is_exit_2_with_the_reason(self):
        git(self.r, "checkout", "-q", "--orphan", "lonely")
        write(self.r, GOLDEN, "[\"no shared history\"]\n")
        git(self.r, "add", "-A")
        git(self.r, "commit", "-q", "-m", "an unrelated root")
        head = git(self.r, "rev-parse", "HEAD")
        git(self.r, "checkout", "-q", "main")
        r = self.run_check(head)
        self.assertEqual(r.returncode, 2, r.stdout + r.stderr)
        self.assertIn("no merge base", r.stderr)

    def test_a_ref_that_is_not_a_commit_is_exit_2(self):
        r = self.run_check("no-such-ref")
        self.assertEqual(r.returncode, 2, r.stdout + r.stderr)


if __name__ == "__main__":
    unittest.main()
