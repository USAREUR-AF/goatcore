"""goat-processors-runner-deploy.yml refuses before it installs anything.

The runner's play (gate J24) installs a package that runs scripts as root, from
a file the release-file role put in place. Two holds come before that, and each
is run here for real on localhost, with become off, against the test's own
directories (nothing outside them is written, nothing is installed):

  - a goatnet that supplies no runner configuration, an empty one, one that is
    not a mapping, or TLS files that are not a mapping of three PEM texts, is refused at the first
    task with GB-395, and the release-file role never runs;
  - a file that `current` names is installed only when it is the verified
    goat-processors package, in the directory named for its own SHA-256, and
    neither it nor a directory above it can be changed by anyone but its owner:
    another name, or a file its group can write, is refused before apt; the
    verified file goes on to apt.

ops/e2e/runner-release.sh deploys the published releases to a VM, to their
health check, through both drivers.

Run: python3 -m pytest tests/test_goat_processors_runner_play.py
"""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

import yaml

ROOT = Path(__file__).resolve().parents[1]
PLAY = ROOT / "ops/ansible/goat-processors-runner-deploy.yml"
GROUP = "zagros_hosts"
PIN = "example-org/example-tool@runner-v0.8.0@sha256:" + "a" * 64
CONFIG = {"name": "runner-1", "site": "zagros", "inputs": {"udp": [{"listen": "127.0.0.1:7600"}]},
          "pipeline": [{"function": "correlate", "pack": "correlate-assign-time"}]}


def require_ansible_playbook(case):
    if shutil.which("ansible-playbook"):
        return
    if os.environ.get("GOAT_TEST_REQUIRE_ANSIBLE") == "1":
        case.fail("GOAT_TEST_REQUIRE_ANSIBLE=1 but ansible-playbook is not on PATH")
    case.skipTest("ansible-playbook is required")


class RunnerPlay(unittest.TestCase):

    def run_play(self, group_vars, file_name=None, mode="0644"):
        """Run the play on localhost in GROUP. With `file_name`, a stand-in
        package of that name is handed to the role as the verified file."""
        require_ansible_playbook(self)
        d = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, d)
        inv = d / "inventory"
        (inv / "group_vars" / GROUP).mkdir(parents=True)
        (inv / "hosts.ini").write_text(f"[{GROUP}]\nlocalhost ansible_connection=local\n")
        (inv / "group_vars" / GROUP / "vars.yml").write_text(yaml.safe_dump(group_vars))
        extra = {
            "target": GROUP, "ansible_become": False,
            "ansible_python_interpreter": "{{ ansible_playbook_python }}",
            "release_file_component": "goat-processors-runner",
            "release_file_root": str(d / "root"),
            "release_file_owner": subprocess.run(["id", "-un"], capture_output=True, text=True).stdout.strip(),
            "release_file_group": subprocess.run(["id", "-gn"], capture_output=True, text=True).stdout.strip(),
            "rollback_ledger_dir": str(d / "ledger"),
            "rollback_assembled_marker": str(d / "no-marker"),
            "release_file_mode": mode,
        }
        extra["rollback_ledger_owner"], extra["rollback_ledger_group"] = extra["release_file_owner"], extra["release_file_group"]
        if file_name:
            src = d / "src" / file_name
            src.parent.mkdir()
            src.write_bytes(b"not a package\n")
            sha = hashlib.sha256(src.read_bytes()).hexdigest()
            extra["release_file_ref"] = PIN
            extra["release_file_files"] = {a: {"src": str(src), "name": file_name, "sha256": sha} for a in ("amd64", "arm64")}
        env = dict(os.environ, ANSIBLE_NOCOLOR="1", ANSIBLE_LOCALHOST_WARNING="False",
                   ANSIBLE_BECOME="False", ANSIBLE_RETRY_FILES_ENABLED="False", ANSIBLE_CACHE_PLUGIN="memory")
        env.pop("ANSIBLE_CONFIG", None)
        r = subprocess.run(["ansible-playbook", "-i", str(inv / "hosts.ini"), str(PLAY), "-e", json.dumps(extra)],
                           cwd=d, env=env, capture_output=True, text=True, timeout=300)
        return r, d

    def test_no_configuration_is_refused_before_anything_is_installed(self):
        for gv in ({}, {"goat_processors_runner_config": {}}, {"goat_processors_runner_config": "name: x"},
                   {"goat_processors_runner_config": CONFIG, "goat_processors_runner_tls": "ca.pem"},
                   {"goat_processors_runner_config": CONFIG, "goat_processors_runner_tls": {"ca": "PEM", "certificate": "PEM"}},
                   {"goat_processors_runner_config": CONFIG, "goat_processors_runner_tls": {"ca": "PEM", "certificate": "PEM", "private_key": ""}}):
            with self.subTest(gv):
                r, d = self.run_play(gv, file_name="goat-processors_0.8.0_amd64.deb")
                self.assertEqual(r.returncode, 2, r.stdout[-3000:])
                self.assertIn("GB-395 the goat-processors runner's configuration is not supplied", r.stdout)
                self.assertNotIn("release-file |", r.stdout, "the role ran before the refusal")
                self.assertFalse((d / "root").exists() or (d / "ledger").exists())

    def test_a_file_that_is_not_a_goat_processors_package_is_not_installed(self):
        r, d = self.run_play({"goat_processors_runner_config": CONFIG}, file_name="tool.deb")
        self.assertNotEqual(r.returncode, 0, r.stdout[-3000:])
        self.assertIn("is not a verified goat-processors package", r.stdout, r.stdout[-3000:])
        self.assertNotIn("goat-processors runner | 2 install the package", r.stdout.split("is not a verified")[1])
        # The role did put it in place: the hold is the play's own, after it.
        self.assertTrue((d / "root" / "goat-processors-runner" / "current").is_symlink())

    def test_the_verified_package_passes_the_hold_and_goes_on_to_apt(self):
        # A stand-in under the package's name: held, accepted, and then refused by
        # apt (it is no package, and this run is not root), never by the hold.
        r, d = self.run_play({"goat_processors_runner_config": CONFIG}, file_name="goat-processors_0.8.0_amd64.deb")
        self.assertNotEqual(r.returncode, 0)
        self.assertNotIn("is not a verified goat-processors package", r.stdout, r.stdout[-3000:])
        self.assertIn("goat-processors runner | 2 install the package", r.stdout)

    def test_a_package_someone_else_can_change_is_not_installed(self):
        r, d = self.run_play({"goat_processors_runner_config": CONFIG}, file_name="goat-processors_0.8.0_amd64.deb",
                             mode="0664")
        self.assertIn("or someone other than", r.stdout, r.stdout[-3000:])
        self.assertNotIn("goat-processors runner | 2 install the package", r.stdout.split("or someone other than")[1])


if __name__ == "__main__":
    unittest.main()
