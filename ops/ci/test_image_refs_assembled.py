#!/usr/bin/env python3
"""check-image-refs-pinned.py on an assembled tree.

An assembled tree holds its layers uncommitted over goatcore's HEAD. The guard
judges the layers' references and accepts their allow.d lines there, and a
layer's digest pin is refreshed by that layer's own dependabot file (the
marker's <layer>_path). In a plain checkout untracked files stay out. A role
default of PENDING (the fail-closed placeholder) is not a reference.
"""
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

GUARD = Path(__file__).with_name("check-image-refs-pinned.py").resolve()
FLOATING = 'demo_image: "example.invalid/demo:latest"\n'
ALLOW_LINE = "ops/ansible/roles/demo/defaults/main.yml demo_image   # fixture\n"


class AssembledTree(unittest.TestCase):
    def tree(self, assembled, overlay_allow):
        root = Path(tempfile.mkdtemp())
        self.addCleanup(subprocess.run, ["rm", "-rf", str(root)])
        env = dict(os.environ, GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_NOSYSTEM="1")
        subprocess.run(["git", "init", "-q", str(root)], check=True, env=env)
        (root / "ops/ci").mkdir(parents=True)
        (root / "ops/ci/floating-image-pins.allow").write_text("# fixture\n")
        subprocess.run(["git", "-C", str(root), "add", "-A"], check=True, env=env)
        subprocess.run(["git", "-C", str(root), "-c", "user.name=t", "-c", "user.email=t@t",
                        "commit", "-q", "-m", "base"], check=True, env=env)
        # The overlay's files: copied over the checkout, never committed there.
        role = root / "ops/ansible/roles/demo/defaults"
        role.mkdir(parents=True)
        (role / "main.yml").write_text(FLOATING)
        if overlay_allow:
            (root / "ops/ci/floating-image-pins.allow.d").mkdir()
            (root / "ops/ci/floating-image-pins.allow.d/overlay.allow").write_text(ALLOW_LINE)
        if assembled:
            (root / ".goat-assembled").write_text("goatcore_ref=0\noverlay_commit=0\n")
        return root, env

    def run_guard(self, root, env):
        return subprocess.run([sys.executable, str(GUARD)], cwd=root, env=env,
                              capture_output=True, text=True)

    def test_assembled_tree_judges_an_untracked_overlay_reference(self):
        r = self.run_guard(*self.tree(assembled=True, overlay_allow=False))
        self.assertNotEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn("ops/ansible/roles/demo/defaults/main.yml", r.stdout + r.stderr)

    def test_assembled_tree_accepts_the_overlay_allow_line(self):
        r = self.run_guard(*self.tree(assembled=True, overlay_allow=True))
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertNotIn("STALE", r.stdout + r.stderr)

    def test_pending_placeholder_is_not_a_reference(self):
        root, env = self.tree(assembled=True, overlay_allow=False)
        (root / "ops/ansible/roles/demo/defaults/main.yml").write_text('demo_image: "PENDING"\n')
        r = self.run_guard(root, env)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)

    def pinned_dockerfile(self, root):
        d = root / "ops/base-images/demo-build"
        d.mkdir(parents=True)
        (d / "Dockerfile").write_text("FROM example.invalid/base:1@sha256:" + "0" * 64 + "\n")

    def test_a_layer_pin_needs_a_dependabot_directory(self):
        root, env = self.tree(assembled=True, overlay_allow=True)
        self.pinned_dockerfile(root)
        r = self.run_guard(root, env)
        self.assertNotEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn("NO REFRESH", r.stdout + r.stderr)

    def test_a_layer_dependabot_file_refreshes_its_pin(self):
        root, env = self.tree(assembled=True, overlay_allow=True)
        self.pinned_dockerfile(root)
        layer = root.parent / (root.name + "-layer")
        (layer / ".github").mkdir(parents=True)
        self.addCleanup(subprocess.run, ["rm", "-rf", str(layer)])
        (layer / ".github/dependabot.yml").write_text(
            "version: 2\nupdates:\n  - package-ecosystem: docker\n    directories:\n      - /ops/base-images/demo-build\n")
        with open(root / ".goat-assembled", "a") as marker:
            marker.write(f"goatstuff_path={layer}\n")
        r = self.run_guard(root, env)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)

    def test_plain_checkout_leaves_untracked_files_out(self):
        r = self.run_guard(*self.tree(assembled=False, overlay_allow=False))
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)


if __name__ == "__main__":
    unittest.main()
