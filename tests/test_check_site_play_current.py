#!/usr/bin/env python3
"""A site's own copy of a generic play is refused once the generic play moved.

The drivers run a site's own `<site>-<suffix>` (for example
`<site>-seed-from-peer.yml`) in place of the generic `site-<suffix>` whenever
the deployment keeps one (crates/goatbuilder/src/standup.rs site_play). A copy
made from an older generic play misses every later fix, and nothing recorded
what it was made from. ops/scripts/check-site-play-current.py:

- passes when the site keeps no copy (the generic play runs);
- passes a wrapper that imports the generic play (it cannot go stale);
- passes a copy whose `# derived-from: site-<suffix> sha256:<hex>` line names
  the current generic play's digest;
- refuses, with GB-362 and exit 1, a copy with no such line or with another
  digest, naming both digests.
"""
import hashlib
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
CHECK = ROOT / "ops/scripts/check-site-play-current.py"
SUFFIX = "seed-from-peer.yml"
GENERIC = "---\n- name: generic seed\n  hosts: all\n  tasks: []\n"


def digest(text):
    return hashlib.sha256(text.encode()).hexdigest()


class SitePlayCurrent(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.repo = Path(self.tmp.name)
        (self.repo / "ops/ansible").mkdir(parents=True)
        (self.repo / f"ops/ansible/site-{SUFFIX}").write_text(GENERIC)

    def tearDown(self):
        self.tmp.cleanup()

    def own(self, text):
        (self.repo / f"ops/ansible/zagros-{SUFFIX}").write_text(text)

    def run_check(self):
        return subprocess.run([sys.executable, str(CHECK), "--repo", str(self.repo), "zagros", SUFFIX],
                              capture_output=True, text=True)

    def test_no_site_copy_passes(self):
        out = self.run_check()
        self.assertEqual(out.returncode, 0, out.stderr)
        self.assertIn("no site copy", out.stdout)

    def test_a_wrapper_that_imports_the_generic_play_passes(self):
        self.own(f"---\n- name: zagros seed\n  ansible.builtin.import_playbook: site-{SUFFIX}\n  vars:\n    site_id: zagros\n")
        out = self.run_check()
        self.assertEqual(out.returncode, 0, out.stderr)
        self.assertIn("imports", out.stdout)

    def test_a_copy_derived_from_the_current_generic_play_passes(self):
        self.own(f"# derived-from: site-{SUFFIX} sha256:{digest(GENERIC)}\n---\n- name: zagros seed\n  hosts: all\n  tasks: []\n")
        out = self.run_check()
        self.assertEqual(out.returncode, 0, out.stderr)

    def test_a_copy_with_no_derived_from_line_is_refused(self):
        self.own("---\n- name: zagros seed\n  hosts: all\n  tasks: []\n")
        out = self.run_check()
        self.assertEqual(out.returncode, 1, out.stdout)
        self.assertIn("GB-362", out.stderr)
        self.assertIn(digest(GENERIC), out.stderr)
        self.assertIn("derived-from", out.stderr)

    def test_a_copy_derived_from_an_older_generic_play_is_refused(self):
        old = digest("an older generic play")
        self.own(f"# derived-from: site-{SUFFIX} sha256:{old}\n---\n- name: zagros seed\n  hosts: all\n  tasks: []\n")
        out = self.run_check()
        self.assertEqual(out.returncode, 1, out.stdout)
        self.assertIn("GB-362", out.stderr)
        self.assertIn(old, out.stderr)
        self.assertIn(digest(GENERIC), out.stderr)


if __name__ == "__main__":
    unittest.main()
