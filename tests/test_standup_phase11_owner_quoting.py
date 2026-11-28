#!/usr/bin/env python3
"""Phase 11's step takes the owner address as data (ADR 1274).

goat-standup.sh builds the phase 11 command from its own text. On a goatlite
the declared admin address is passed as "$(printenv GOAT_BOOTSTRAP_OWNER_EMAIL)",
as the bootstrap phases pass it, so the phase's shell never parses the address;
goatbuilder standup plans the same word (owner_word). The address below has a
quote and a command substitution in it: the planned command must hand it over
unchanged and run nothing.
"""
import os
from pathlib import Path
import re
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "ops/scripts/goat-standup.sh"


def planned_line(text, admin, marker):
    """The phase 11 step's command as goat-standup.sh plans it on a goatlite
    founder, with `step` printing the command instead of running it."""
    owner_block = text[text.index('OWNER_EMAIL_STANDUP="\\$OWNER_EMAIL"'):]
    owner_block = owner_block[:owner_block.index("OWNER_EMAIL_PH11=\"$OWNER_EMAIL_ARG\"; fi\n") + len("OWNER_EMAIL_PH11=\"$OWNER_EMAIL_ARG\"; fi\n")]
    steps = text[text.index("phase11_mint_steps() {"):]
    steps = steps[:steps.index("\n}\n") + 3]
    prog = "\n".join([
        "set -eu",
        "step() { printf '%s\\n' \"$4\"; }",
        "REPO=/r; SITE=pasture; GROUP=pasture_hosts; PROFILE=lan; LITE=1",
        'lite_admin="$1"',
        owner_block,
        steps,
        "phase11_mint_steps",
    ])
    out = subprocess.run(["bash", "-c", prog, "plan", admin], capture_output=True, text=True, check=True).stdout
    return out.splitlines()[0]


class OwnerAsData(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.dir = Path(self._tmp.name)
        self.marker = self.dir / "touched"
        # A quote, then a command substitution between two quotes: inside the
        # old single quotes, the substitution lands outside them.
        self.admin = "o'brien+'$(touch " + str(self.marker) + ")'@example.org"
        self.breaks_out = "a'$(touch " + str(self.marker) + ")'@example.org"

    def tearDown(self):
        self._tmp.cleanup()

    def run_owner_word(self, line):
        """Run the planned --owner word as the phase's shell would, and return
        what it hands over."""
        word = re.search(r"--owner (.*)$", line).group(1)
        r = subprocess.run(["bash", "-c", f"printf '%s' {word}"], capture_output=True, text=True,
                           env=dict(os.environ, GOAT_BOOTSTRAP_OWNER_EMAIL=self.admin))
        return r, word

    def test_the_planned_owner_is_handed_over_unchanged_and_runs_nothing(self):
        for admin in (self.admin, self.breaks_out):
            self.admin = admin
            line = planned_line(SCRIPT.read_text(), admin, self.marker)
            r, word = self.run_owner_word(line)
            self.assertEqual(r.stdout, admin)
            self.assertFalse(self.marker.exists(), "the address was parsed by the shell")
        self.assertFalse(self.marker.exists(), "the address was parsed by the shell")
        self.assertNotIn(self.admin, line, "the address is not on the command line")
        # goatbuilder standup's owner_word for a declared address, the same word.
        self.assertEqual(word, '"$(printenv GOAT_BOOTSTRAP_OWNER_EMAIL)"')

    def test_the_old_text_would_have_run_the_address(self):
        old = SCRIPT.read_text().replace("--owner $OWNER_EMAIL_PH11", "--owner '$OWNER_EMAIL_STANDUP'")
        self.assertNotEqual(old, SCRIPT.read_text())
        line = planned_line(old, self.breaks_out, self.marker)
        self.admin = self.breaks_out
        self.run_owner_word(line)
        self.assertTrue(self.marker.exists(), "the quoting this test guards against runs the address")


if __name__ == "__main__":
    unittest.main()


class UndeclaredOwner(unittest.TestCase):
    def test_an_undeclared_owner_is_the_quoted_placeholder_and_needs_no_variable(self):
        text = SCRIPT.read_text()
        owner_block = text[text.index('OWNER_EMAIL_STANDUP="\\$OWNER_EMAIL"'):]
        owner_block = owner_block[:owner_block.index('OWNER_EMAIL_PH11="$OWNER_EMAIL_ARG"; fi\n') + len('OWNER_EMAIL_PH11="$OWNER_EMAIL_ARG"; fi\n')]
        steps = text[text.index("phase11_mint_steps() {"):]
        steps = steps[:steps.index("\n}\n") + 3]
        prog = "\n".join(["set -eu", "step() { printf '%s\\n' \"$4\"; }", "REPO=/r; SITE=zagros; GROUP=zagros_hosts; PROFILE=lan; LITE=0",
                          owner_block, steps, "phase11_mint_steps"])
        line = subprocess.run(["bash", "-c", prog], capture_output=True, text=True, check=True).stdout.splitlines()[0]
        self.assertTrue(line.endswith("--owner '$OWNER_EMAIL'"), line)
