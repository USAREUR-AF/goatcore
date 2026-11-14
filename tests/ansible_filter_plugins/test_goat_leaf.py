#!/usr/bin/env python3
"""Tests for goat_leaf (#2165): a fresh worktree finds a site's leaf.

The unit cases call the filter directly. The last case builds a throwaway git
repository with this filter, adds a brand-new worktree to it, and runs a real
ansible-playbook from that worktree: the play must stat the leaf that sits only
in the per-operator state dir, which is what a router deploy from a fresh
checkout needs.

    python3 tests/ansible_filter_plugins/test_goat_leaf.py
"""

import importlib.util
import os
import shutil
import site
import subprocess
import sys
import tempfile
import textwrap
import unittest
import unittest.mock as mock
from pathlib import Path

FILTERS = Path(__file__).resolve().parents[2] / "ops" / "ansible" / "filter_plugins"
spec = importlib.util.spec_from_file_location("goat_leaf", FILTERS / "goat_leaf.py")
gl = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gl)


def git(*args, cwd):
    subprocess.run(["git", *args], cwd=cwd, check=True, capture_output=True,
                   env={**os.environ, "GIT_AUTHOR_NAME": "t", "GIT_AUTHOR_EMAIL": "t@t",
                        "GIT_COMMITTER_NAME": "t", "GIT_COMMITTER_EMAIL": "t@t"})


class Unit(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp())
        self.home = self.tmp / "home"
        self.state = self.home / ".goat" / "state" / "zz-test-leaves"
        self.env = mock.patch.dict(os.environ, {"HOME": str(self.home)}, clear=False)
        self.env.start()
        os.environ.pop("GOAT_LEAF_DIR", None)
        self.ops = self.tmp / "repo" / "ops"
        (self.ops / "enrollment" / "manifests").mkdir(parents=True)
        self.patch = mock.patch.multiple(gl, OPS=self.ops, REPO=self.ops.parent,
                                         _main_checkout=lambda: None)
        self.patch.start()

    def tearDown(self):
        self.patch.stop()
        self.env.stop()
        shutil.rmtree(self.tmp)

    def put(self, d, name="router-cert.pem"):
        d.mkdir(parents=True, exist_ok=True)
        (d / name).write_text("CERT\n")
        return str(d / name)

    def test_state_dir_wins(self):
        want = self.put(self.state)
        self.put(self.ops / "enrollment" / "zz-test")
        self.assertEqual(gl.goat_leaf("router-cert.pem", "zz-test"), want)

    def test_checkout_copy_is_the_fallback(self):
        want = self.put(self.ops / "enrollment" / "zz-test")
        self.assertEqual(gl.goat_leaf("router-cert.pem", "zz-test"), want)

    def test_main_checkout_copy_is_seen_from_a_worktree(self):
        main = self.tmp / "main"
        want = self.put(main / "ops" / "enrollment" / "zz-test")
        with mock.patch.object(gl, "_main_checkout", lambda: main):
            self.assertEqual(gl.goat_leaf("router-cert.pem", "zz-test"), want)

    def test_missing_names_the_state_dir(self):
        self.assertEqual(gl.goat_leaf("router-cert.pem", "zz-test"),
                         str(self.state / "router-cert.pem"))

    def test_manifest_out_dir_is_honoured(self):
        other = self.tmp / "elsewhere"
        (self.ops / "enrollment" / "manifests" / "zz-test-certs.yaml").write_text(
            f"site_id: zz-test\nout_dir: {other}   # comment\n")
        want = self.put(other)
        self.assertEqual(gl.goat_leaf("router-cert.pem", "zz-test"), want)

    def test_declared_state_root_is_honoured(self):
        """A caller outside a play gets the root group_vars/all.yml declares."""
        (self.ops / "ansible" / "group_vars").mkdir(parents=True)
        (self.ops / "ansible" / "group_vars" / "all.yml").write_text(
            "domain: x\ngoat_controller_state_dir: \"{{ lookup('env', 'HOME') }}/.elsewhere/state\"\n")
        want = self.put(self.home / ".elsewhere" / "state" / "zz-test-leaves")
        self.put(self.state)
        self.assertEqual(gl.goat_leaf("router-cert.pem", "zz-test"), want)

    def test_root_argument_wins(self):
        """Inside a play the filter passes the play's goat_controller_state_dir."""
        want = self.put(self.tmp / "root" / "zz-test-leaves")
        self.put(self.state)
        self.assertEqual(gl.goat_leaf("router-cert.pem", "zz-test", str(self.tmp / "root")), want)

    def test_override_dir(self):
        want = self.put(self.tmp / "override")
        os.environ["GOAT_LEAF_DIR"] = str(self.tmp / "override")
        try:
            self.assertEqual(gl.goat_leaf("router-cert.pem", "zz-test"), want)
        finally:
            del os.environ["GOAT_LEAF_DIR"]

    def test_site_slash_file_form(self):
        want = self.put(self.state, "names-collector-cert.pem")
        self.assertEqual(gl.goat_leaf("zz-test/names-collector-cert.pem"), want)

    def test_mint_command(self):
        self.assertEqual(gl.goat_leaf_mint("router-key.pem", "zz-test"),
                         "ops/scripts/mint-site-certs.sh zz-test --only router")
        self.assertEqual(gl.goat_leaf_mint("zz-test/dashboard-a-cert.pem"),
                         "ops/scripts/mint-site-certs.sh zz-test --only dashboard-a")


@unittest.skipUnless(shutil.which("ansible-playbook"), "ansible-playbook not installed")
class FreshWorktreeDeploy(unittest.TestCase):
    """A play run from a brand-new worktree finds a leaf held only in the state dir."""

    def test_fresh_worktree_finds_the_leaf(self):
        tmp = Path(tempfile.mkdtemp())
        try:
            repo = tmp / "repo"
            (repo / "ops" / "ansible" / "filter_plugins").mkdir(parents=True)
            (repo / "ops" / "enrollment" / "manifests").mkdir(parents=True)
            shutil.copy(FILTERS / "goat_leaf.py", repo / "ops" / "ansible" / "filter_plugins")
            (repo / "ops" / "ansible" / "ansible.cfg").write_text(
                "[defaults]\nfilter_plugins = ./filter_plugins\n")
            (repo / "ops" / "ansible" / "group_vars").mkdir(parents=True)
            # the state root a deployment declares; the filter reads it as a play variable
            (repo / "ops" / "ansible" / "group_vars" / "all.yml").write_text(
                "goat_controller_state_dir: \"{{ lookup('env', 'HOME') }}/.elsewhere/state\"\n")
            (repo / "ops" / "ansible" / "probe.yml").write_text(textwrap.dedent("""\
                - hosts: localhost
                  gather_facts: false
                  vars:
                    leaf: "{{ 'router-cert.pem' | goat_leaf('zz-test') }}"
                  tasks:
                    - ansible.builtin.stat: {path: "{{ leaf }}"}
                      register: st
                    - ansible.builtin.assert:
                        that: [st.stat.exists]
                        fail_msg: "not found: {{ leaf }}"
                """))
            (repo / ".gitignore").write_text("*.pem\n")
            git("init", "-q", "-b", "main", cwd=repo)
            git("add", "-A", cwd=repo)
            git("commit", "-qm", "init", cwd=repo)
            wt = tmp / "fresh"
            git("worktree", "add", "-q", str(wt), cwd=repo)
            home = tmp / "home"
            state = home / ".elsewhere" / "state" / "zz-test-leaves"
            state.mkdir(parents=True)
            (state / "router-cert.pem").write_text("CERT\n")
            # A fake HOME hides a pip --user ansible; keep the real user site visible.
            env = {**os.environ, "HOME": str(home), "PYTHONUSERBASE": site.getuserbase(), "ANSIBLE_CONFIG": str(wt / "ops/ansible/ansible.cfg"),
                   "ANSIBLE_LOCALHOST_WARNING": "False", "ANSIBLE_INVENTORY_UNPARSED_WARNING": "False"}
            env.pop("GOAT_LEAF_DIR", None)
            r = subprocess.run(["ansible-playbook", "-i", "localhost,", "-c", "local", "probe.yml"],
                               cwd=wt / "ops" / "ansible", env=env, capture_output=True, text=True)
            self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        finally:
            shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(0 if unittest.main(exit=False, verbosity=2).result.wasSuccessful() else 1)
