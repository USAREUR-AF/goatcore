#!/usr/bin/env python3
"""generate-envrc-example.py reads an assembled tree's untracked layer files.

An assembled tree holds its layers uncommitted over goatcore's HEAD. A
variable that only a layer's file reads belongs in that tree's .envrc.example;
in a plain checkout untracked files stay out, as before.
"""
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

GENERATOR = Path(__file__).with_name("generate-envrc-example.py").resolve()


class AssembledTree(unittest.TestCase):
    def generate(self, assembled):
        root = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, root, True)
        env = dict(os.environ, GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_NOSYSTEM="1")
        subprocess.run(["git", "init", "-q", str(root)], check=True, env=env)
        (root / "ops/scripts").mkdir(parents=True)
        shutil.copy(GENERATOR, root / "ops/scripts/generate-envrc-example.py")
        (root / "ops/scripts/core_tool.py").write_text('import os\nos.environ.get("CORE_ONLY_VAR")\n')
        subprocess.run(["git", "-C", str(root), "add", "-A"], check=True, env=env)
        subprocess.run(["git", "-C", str(root), "-c", "user.name=t", "-c", "user.email=t@t",
                        "commit", "-q", "-m", "base"], check=True, env=env)
        # A layer's file: copied over the checkout, never committed there.
        (root / "ops/scripts/layer_tool.py").write_text('import os\nos.environ.get("LAYER_ONLY_VAR")\n')
        if assembled:
            (root / ".goat-assembled").write_text("goatcore_ref=0\noverlay_commit=0\n")
        r = subprocess.run([sys.executable, str(root / "ops/scripts/generate-envrc-example.py")],
                           cwd=root, env=env, capture_output=True, text=True)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        return (root / ".envrc.example").read_text()

    def test_assembled_tree_documents_a_layer_variable(self):
        text = self.generate(assembled=True)
        self.assertIn("CORE_ONLY_VAR", text)
        self.assertIn("LAYER_ONLY_VAR", text)

    def test_plain_checkout_leaves_untracked_files_out(self):
        text = self.generate(assembled=False)
        self.assertIn("CORE_ONLY_VAR", text)
        self.assertNotIn("LAYER_ONLY_VAR", text)


if __name__ == "__main__":
    unittest.main()
