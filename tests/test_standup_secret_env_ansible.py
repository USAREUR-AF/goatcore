#!/usr/bin/env python3
"""Standup passes secrets through the environment, not the command line, and
Ansible does not print them.

The `-e` words are taken from a recorded standup plan
(crates/goatbuilder/tests/fixtures/full-plans.golden.json): phase 5's
site-bootstrap line, as `goatbuilder standup` prints it. They are handed to a
real `ansible-playbook -vvv`, run on this machine (connection local, no become)
from a directory of its own, never from ops/ansible, with a synthetic value in
each variable's environment. The play writes the values to a file with no_log,
as the consuming tasks must (the test fails without it). The values reach the play, and none of them
appears in anything Ansible prints.
"""
import json
import os
import shlex
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
GOLDEN = ROOT / "crates/goatbuilder/tests/fixtures/full-plans.golden.json"
NAMES = ("postgres_superuser_password", "netbird_encryption_key", "pgbackrest_cipher_pass")


def bootstrap_line():
    plans = json.loads(GOLDEN.read_text())
    for rows in plans.values():
        for r in rows:
            d = r.get("detail", "")
            if r.get("name") == "cmd" and " site-bootstrap.yml " in d and "postgres_superuser_password" in d:
                return d
    raise AssertionError("no recorded site-bootstrap line")


def secret_extra_vars(line):
    """The `-e` words of `line` that carry one of NAMES, as the shell hands them over."""
    words = shlex.split(line)
    return [words[i + 1] for i, w in enumerate(words[:-1]) if w == "-e" and words[i + 1].split("=", 1)[0] in NAMES]


PLAY = """\
- hosts: localhost
  connection: local
  gather_facts: false
  tasks:
    # lineinfile, unlike copy's content, has no argument Ansible hides itself:
    # without no_log, -vvv prints each line it was given.
    - name: write the values where only the test reads them
      ansible.builtin.lineinfile:
        path: "{{ out }}"
        create: true
        mode: "0600"
        line: "{{ item }}"
      loop:
        - "{{ postgres_superuser_password }}"
        - "{{ netbird_encryption_key }}"
        - "{{ pgbackrest_cipher_pass }}"
      no_log: true
    - name: a task that prints something
      ansible.builtin.debug:
        msg: "the play ran"
"""


class SecretsThroughTheEnvironment(unittest.TestCase):
    def test_the_recorded_line_reads_each_secret_from_the_environment(self):
        ev = secret_extra_vars(bootstrap_line())
        self.assertEqual(len(ev), 3, ev)
        for word in ev:
            name, value = word.split("=", 1)
            self.assertRegex(value, r'^\{\{ lookup\("env", "[A-Z0-9_]+"\) \}\}$', f"{name} is read by the play, not expanded by the shell")

    @unittest.skipUnless(shutil.which("ansible-playbook") or os.environ.get("GOAT_TEST_REQUIRE_ANSIBLE") == "1",
                         "ansible-playbook is not installed (CI sets GOAT_TEST_REQUIRE_ANSIBLE=1)")
    def test_ansible_at_vvv_receives_the_values_and_prints_none(self):
        ev = secret_extra_vars(bootstrap_line())
        values = {}
        env = {k: v for k, v in os.environ.items() if not k.startswith("ANSIBLE_")}
        for word in ev:
            var = word.split('"env", "', 1)[1].split('"', 1)[0]
            values[var] = f"SYNTHETIC-{var}-7f3a9c"
            env[var] = values[var]
        with tempfile.TemporaryDirectory() as d:
            play = Path(d) / "play.yml"
            play.write_text(PLAY)
            out = Path(d) / "values"
            env.update({
                "ANSIBLE_BECOME": "False",
                "ANSIBLE_CONFIG": str(Path(d) / "ansible.cfg"),  # none: no checkout config is read
                "ANSIBLE_LOCAL_TEMP": str(Path(d) / "tmp"),
                "ANSIBLE_REMOTE_TMP": str(Path(d) / "remote-tmp"),
                "ANSIBLE_HOME": str(Path(d) / "home"),
                "ANSIBLE_NOCOLOR": "1",
                "ANSIBLE_LOG_PATH": str(Path(d) / "ansible.log"),
            })
            args = ["ansible-playbook", "-vvv", "-i", "localhost,", str(play), "-e", f"out={out}"]
            for word in ev:
                args += ["-e", word]
            r = subprocess.run(args, cwd=d, env=env, capture_output=True, text=True, timeout=300)
            log = Path(d) / "ansible.log"
            printed = r.stdout + r.stderr + (log.read_text(errors="replace") if log.exists() else "")
            self.assertEqual(r.returncode, 0, printed[-3000:])
            self.assertIn("the play ran", printed)
            self.assertEqual(out.read_text().split(), list(values.values()), "each value reached the play from the environment")
            for var, value in values.items():
                self.assertNotIn(value, printed, f"{var}'s value is printed at -vvv")
                self.assertNotIn(value, " ".join(args), f"{var}'s value is on the command line")


if __name__ == "__main__":
    unittest.main()
