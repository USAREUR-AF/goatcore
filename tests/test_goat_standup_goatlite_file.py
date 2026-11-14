#!/usr/bin/env python3
"""ops/scripts/goat-standup.sh writes a declared value into a planned command as one
shell word that the phase's shell passes on as declared (sh_quote, sh_word; standup.rs
has the same two). And it reads <repo>/goatlite.yaml the way `goatbuilder standup`
does (standup.rs resolve_lite): a file it cannot read as YAML refuses the run, and a
file that names this site and does not validate refuses it too. Neither is taken as
"not a goatlite", which planned a different standup from the one goatbuilder plans."""
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]


def checkout(goatlite: str) -> Path:
    """A throwaway tree holding the script, the validator and the schema it reads."""
    root = Path(tempfile.mkdtemp())
    for rel in ("ops/scripts/goat-standup.sh", "ops/scripts/project-setting.sh", "ops/goatbuilder/validate.py"):
        (root / rel).parent.mkdir(parents=True, exist_ok=True)
        shutil.copy(REPO / rel, root / rel)
    shutil.copytree(REPO / "ops/goatbuilder/schema", root / "ops/goatbuilder/schema")
    (root / "goatlite.yaml").write_text(goatlite)
    return root


def plan(root: Path):
    env = {"PATH": os.environ["PATH"], "HOME": tempfile.mkdtemp()}
    return subprocess.run(["bash", str(root / "ops/scripts/goat-standup.sh"), "pasture", "--plan"],
                          capture_output=True, text=True, env=env, timeout=120)


FOUR = "schema: goatbuilder/v1\npreset: lite\nlite:\n  name: pasture\n  host: 192.0.2.50\n  ssh_user: ubuntu\n  admin: me@example.org\n"


class GoatliteFile(unittest.TestCase):
    def test_a_file_that_is_not_yaml_refuses(self):
        r = plan(checkout("schema: goatbuilder/v1\npreset: lite\nlite: [unclosed\n"))
        self.assertNotEqual(r.returncode, 0, r.stdout)
        self.assertIn("goatlite.yaml", r.stderr)

    def test_a_file_naming_this_site_that_does_not_validate_refuses(self):
        r = plan(checkout(FOUR.replace("  admin: me@example.org\n", "")))
        self.assertNotEqual(r.returncode, 0, r.stdout)
        self.assertIn("goatlite.yaml", r.stderr)

    def test_a_file_naming_another_goatnet_is_left_alone(self):
        r = plan(checkout(FOUR.replace("name: pasture", "name: meadow").replace("  admin: me@example.org\n", "")))
        self.assertNotIn("goatlite.yaml does not", r.stderr)


class ShellWords(unittest.TestCase):
    """The script's sh_quote and sh_word, defined as the script defines them, give
    back each declared value when the phase's shell reads the word."""

    def words(self, fn: str, value: str) -> str:
        text = (REPO / "ops/scripts/goat-standup.sh").read_text()
        start = text.index("sh_quote() {")
        defs = text[start:text.index("\n}\n", text.index("sh_word() {")) + 3]
        home = tempfile.mkdtemp()
        prog = defs + f'w="$({fn} "$1")"; eval "printf %s $w"'
        r = subprocess.run(["bash", "-euo", "pipefail", "-c", prog, "probe", value],
                           capture_output=True, text=True, env={"PATH": os.environ["PATH"], "HOME": home})
        self.assertEqual(r.returncode, 0, r.stderr)
        return r.stdout, home

    def test_each_value_comes_back_as_declared(self):
        for v in ["default", "~/.goat/state", "/srv/goat state", "p$(x) 'q\"r`y`\\z", "~/a b$(x)'q/state"]:
            for fn in ("sh_quote", "sh_word"):
                out, home = self.words(fn, v)
                want = v if fn == "sh_quote" or not v.startswith("~/") else home + v[1:]
                self.assertEqual(out, want, f"{fn} {v!r}")

    def test_a_plain_value_is_written_as_it_always_was(self):
        text = (REPO / "ops/scripts/goat-standup.sh").read_text()
        start = text.index("sh_quote() {")
        defs = text[start:text.index("\n}\n", text.index("sh_word() {")) + 3]
        r = subprocess.run(["bash", "-c", defs + 'sh_word "~/.goat/state/pasture-ca/ca.key.age"; echo; sh_word default'],
                           capture_output=True, text=True)
        self.assertEqual(r.stdout.split(), ["~/.goat/state/pasture-ca/ca.key.age", "default"])


if __name__ == "__main__":
    unittest.main()
