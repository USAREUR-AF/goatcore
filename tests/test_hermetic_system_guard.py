#!/usr/bin/env python3
"""ops/e2e/hermetic.sh with hosts changes the machine it runs on: it appends
`127.0.0.1 ghcr.io` to /etc/hosts, trusts a throwaway CA under
/etc/docker/certs.d/ghcr.io, and runs a registry on :443. On a shared machine
that points every session's ghcr.io at an empty local registry.

So the tier refuses to start with hosts unless it runs in CI (GITHUB_ACTIONS=true)
or the caller passes --throwaway-machine, and once it has made a change, it undoes
it on exit (ops/e2e/system.sh). These tests never touch the system: `sudo` and
`docker` are stand-ins on PATH that only record their calls, and the hosts file
and the certificate directory are temporary files (E2E_HOSTS_FILE, E2E_CERTS_DIR).
docs/design/deployment-e2e.md §Hermetic."""
import os
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
HERMETIC = ROOT / "ops/e2e/hermetic.sh"
SYSTEM = ROOT / "ops/e2e/system.sh"
HOSTS_BEFORE = "127.0.0.1 localhost\n::1 ip6-localhost\n"


class Sandbox:
    """A PATH whose sudo and docker only record their calls, a hosts file and a
    certificate directory in a temporary directory, and no GITHUB_ACTIONS."""

    def __init__(self, tmp):
        self.tmp = Path(tmp)
        self.bin = self.tmp / "bin"
        self.bin.mkdir()
        self.log = self.tmp / "calls.log"
        self.hosts = self.tmp / "hosts"
        self.hosts.write_text(HOSTS_BEFORE)
        self.certs = self.tmp / "certs.d"
        self.certs.mkdir()
        for name in ("docker", "sudo-record"):
            p = self.bin / name
            p.write_text('#!/bin/sh\necho "%s $*" >> "%s"\nexit 1\n' % (name, self.log))
            p.chmod(0o755)
        # The sudo stand-in records the call and runs it unprivileged, so the
        # changes land on the temporary files and nowhere else.
        p = self.bin / "sudo"
        p.write_text('#!/bin/sh\necho "sudo $*" >> "%s"\nexec "$@"\n' % self.log)
        p.chmod(0o755)
        self.env = {k: v for k, v in os.environ.items() if k not in ("GITHUB_ACTIONS", "RUNNER_ENVIRONMENT", "E2E_SUDO")}
        self.env.update({"PATH": "%s:%s" % (self.bin, os.environ.get("PATH", "")),
                         "E2E_HOSTS_FILE": str(self.hosts), "E2E_CERTS_DIR": str(self.certs),
                         "HOME": str(self.tmp / "home")})

    def calls(self):
        return self.log.read_text() if self.log.exists() else ""

    def bash(self, script, **extra):
        env = dict(self.env, **extra)
        return subprocess.run(["bash", "-c", ". '%s'\n%s" % (SYSTEM, script)], env=env,
                              capture_output=True, text=True, timeout=60)


class RefusedOnASharedMachine(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.s = Sandbox(self._tmp.name)

    def tearDown(self):
        self._tmp.cleanup()

    def test_with_hosts_and_neither_ci_nor_the_flag_it_refuses_and_changes_nothing(self):
        work = self.s.tmp / "work"
        r = subprocess.run(["bash", str(HERMETIC), "--work", str(work)], env=self.s.env,
                           capture_output=True, text=True, timeout=120)
        self.assertEqual(r.returncode, 2, r.stdout + r.stderr)
        for named in ("/etc/hosts", "/etc/docker/certs.d", ":443", "--throwaway-machine", "--no-hosts"):
            self.assertIn(named, r.stderr, r.stderr)
        self.assertEqual(self.s.calls(), "", "no sudo and no docker before the refusal")
        self.assertEqual(self.s.hosts.read_text(), HOSTS_BEFORE)
        self.assertFalse(work.exists(), "nothing is created before the refusal")

    def test_the_guard_passes_without_hosts_in_ci_and_with_the_flag(self):
        hosted = {"GITHUB_ACTIONS": "true", "RUNNER_ENVIRONMENT": "github-hosted"}
        for hosts, throwaway, extra, want in ((0, 0, {}, 0), (1, 0, hosted, 0),
                                              (1, 1, {}, 0), (1, 0, {}, 2),
                                              (1, 0, {"GITHUB_ACTIONS": "false"}, 2),
                                              # A self-hosted runner is often a shared machine.
                                              (1, 0, {"GITHUB_ACTIONS": "true", "RUNNER_ENVIRONMENT": "self-hosted"}, 2),
                                              (1, 0, {"GITHUB_ACTIONS": "true"}, 2)):
            r = self.s.bash("sys_guard %d %d" % (hosts, throwaway), **extra)
            self.assertEqual(r.returncode, want, (hosts, throwaway, extra, r.stderr))
        self.assertEqual(self.s.calls(), "")


class UndoneOnExit(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.s = Sandbox(self._tmp.name)
        self.ca = self.s.tmp / "ca.pem"
        self.ca.write_text("a throwaway CA\n")

    def tearDown(self):
        self._tmp.cleanup()

    def test_each_change_is_made_through_sudo_and_undone_exactly(self):
        r = self.s.bash("sys_arm_undo; sys_trust_registry '%s'; sys_point_ghcr\n"
                        "grep -qx '127.0.0.1 ghcr.io' \"$E2E_HOSTS_FILE\" || exit 9\n"
                        "[ -f \"$E2E_CERTS_DIR/ghcr.io/ca.crt\" ] || exit 8\n" % self.ca)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(self.s.hosts.read_text(), HOSTS_BEFORE, "the hosts file is byte for byte as before")
        self.assertFalse((self.s.certs / "ghcr.io").exists(), "the directory it made is gone")
        self.assertIn("sudo tee -a %s" % self.s.hosts, self.s.calls())

    def test_a_failing_run_and_a_terminated_run_undo_too(self):
        for ending in ("false", "kill -TERM $$; sleep 5", "kill -INT $$; sleep 5"):
            self.s.hosts.write_text(HOSTS_BEFORE)
            r = self.s.bash("set -e; sys_arm_undo; sys_trust_registry '%s'; sys_point_ghcr; %s" % (self.ca, ending))
            self.assertNotEqual(r.returncode, 0, ending)
            self.assertEqual(self.s.hosts.read_text(), HOSTS_BEFORE, ending)
            self.assertFalse((self.s.certs / "ghcr.io").exists(), ending)

    def test_what_was_there_before_is_left_as_it_was(self):
        before = HOSTS_BEFORE + "127.0.0.1 ghcr.io\n"
        self.s.hosts.write_text(before)
        (self.s.certs / "ghcr.io").mkdir()
        (self.s.certs / "ghcr.io" / "ca.crt").write_text("someone else's CA\n")
        r = self.s.bash("sys_arm_undo; sys_trust_registry '%s'; sys_point_ghcr\n"
                        "[ \"$(cat \"$E2E_CERTS_DIR/ghcr.io/ca.crt\")\" = 'a throwaway CA' ] || exit 7" % self.ca)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(self.s.hosts.read_text(), before, "a line it did not add is not removed")
        self.assertEqual((self.s.certs / "ghcr.io" / "ca.crt").read_text(), "someone else's CA\n",
                         "the certificate it replaced is put back")

    def test_a_ca_that_cannot_be_saved_is_left_and_the_run_stops(self):
        # The CA already there is not readable, so it cannot be saved: the run stops
        # before replacing it, and the undo never writes anything over it.
        (self.s.certs / "ghcr.io").mkdir()
        old = self.s.certs / "ghcr.io" / "ca.crt"
        old.write_text("the CA this machine already trusts\n")
        old.chmod(0o200)
        try:
            r = self.s.bash("set -e; sys_arm_undo; sys_trust_registry '%s'" % self.ca)
        finally:
            old.chmod(0o600)
        self.assertNotEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual(old.read_text(), "the CA this machine already trusts\n",
                         "a CA that could not be saved is never overwritten")


class ARestoreThatFailsIsSaid(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.s = Sandbox(self._tmp.name)
        self.ca = self.s.tmp / "ca.pem"
        self.ca.write_text("a throwaway CA\n")

    def tearDown(self):
        self._tmp.cleanup()

    def test_a_restore_that_fails_names_the_command_and_fails_the_run(self):
        # After the change, sudo fails, as sudo with an expired timestamp and no terminal does.
        script = ("sys_arm_undo; sys_trust_registry '%s'; sys_point_ghcr\n"
                  "cat > '%s' <<'EOF'\n#!/bin/sh\necho \"sudo $*\" >> '%s'\nexit 1\nEOF\n"
                  "exit 0" % (self.ca, self.s.bin / "sudo", self.s.log))
        r = self.s.bash(script)
        self.assertNotEqual(r.returncode, 0, "a run whose restore failed does not exit 0")
        self.assertIn("NOT RESTORED", r.stderr, r.stderr)
        self.assertIn(str(self.s.hosts), r.stderr, "the message names the hosts file to fix")
        self.assertIn(str(self.s.certs / "ghcr.io"), r.stderr, "and the certificate directory")

    def test_a_hosts_file_that_cannot_be_read_is_never_emptied(self):
        r = self.s.bash("sys_arm_undo; sys_point_ghcr\nchmod 0200 \"$E2E_HOSTS_FILE\"")
        self.s.hosts.chmod(0o600)
        self.assertNotEqual(self.s.hosts.read_text(), "", "a read error is not taken for an empty file")
        self.assertIn("NOT RESTORED", r.stderr, r.stderr)

    def test_sudo_must_need_no_password_not_merely_be_cached(self):
        # `sudo -k -n true` succeeds only where sudo needs no password at all; a cached
        # timestamp could expire before the undo runs.
        p = self.s.bin / "sudo"
        p.write_text('#!/bin/sh\n[ "$1" = -k ] && [ "$2" = -n ] && exit 1\nexec "$@"\n')
        p.chmod(0o755)
        r = self.s.bash("sys_sudo_ready")
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("NOPASSWD", r.stderr, r.stderr)
        p.write_text('#!/bin/sh\n[ "$1" = -k ] && shift\n[ "$1" = -n ] && shift\nexec "$@"\n')
        r = self.s.bash("sys_sudo_ready")
        self.assertEqual(r.returncode, 0, r.stderr)


if __name__ == "__main__":
    unittest.main()
