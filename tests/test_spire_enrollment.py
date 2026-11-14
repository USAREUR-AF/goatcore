"""Execute SPIRE's enrollment tasks with real Ansible and a recording Docker boundary.

Only host ownership, package installation and wait duration are adapted for unprivileged temporary
files. Task commands, state paths, conditions and templates remain source-owned.
Run: python3 -m unittest discover -s tests -p test_spire_enrollment.py -v
"""
from pathlib import Path
import json
import os
import shutil
import subprocess
import tempfile
import unittest

import yaml

REPO = Path(__file__).resolve().parents[1]
ROLE = REPO / "ops/ansible/roles/spire"


class Enrollment(unittest.TestCase):
    def setUp(self):
        if not shutil.which("ansible-playbook"):
            if os.environ.get("GOAT_TEST_REQUIRE_ANSIBLE"):
                self.fail("ansible-playbook is required")
            self.skipTest("ansible-playbook unavailable")
        self.tmp = tempfile.TemporaryDirectory(prefix="spire-enrollment-")
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        cert = self.root / 'fixture-ca.pem'
        subprocess.run(['openssl', 'req', '-x509', '-newkey', 'ed25519', '-nodes',
                        '-keyout', str(self.root / 'fixture-key.pem'), '-out', str(cert),
                        '-days', '1', '-subj', '/CN=fixture-ca',
                        '-addext', 'basicConstraints=critical,CA:TRUE'],
                       check=True, capture_output=True, timeout=10)
        self.calls = self.root / "calls.jsonl"
        self.state = self.root / "data/agent"
        self.state.mkdir(parents=True)
        (self.root / "compose").mkdir()
        (self.root / "bin").mkdir()
        role = self.root / "roles/spire"
        role.mkdir(parents=True)
        shutil.copytree(ROLE / "tasks", role / "tasks")
        shutil.copytree(ROLE / "templates", role / "templates")
        # The source copy task resolves this relative to the playbook directory.
        dest = self.root / "compose/mgmt-host"
        dest.mkdir()
        shutil.copyfile(REPO / "ops/compose/mgmt-host/docker-compose.spire-agent.yml",
                        dest / "docker-compose.spire-agent.yml")
        (self.root / "play").mkdir()
        self.vars = {
            "spire_agent_only": True, "spire_enabled": False,
            "spire_agent_server_address": "ca.example.org", "spire_agent_server_port": 8081,
            "spire_agent_node_path": "agent/fixture", "spire_trust_domain": "example.org",
            "spire_agent_image": "ghcr.io/spiffe/spire-agent:1.16.0@sha256:" + "1" * 64,
            "spire_data_dir": str(self.root / "data"),
            "spire_socket_dir": str(self.root / "sockets"),
            "spire_config_dir": str(self.root / "config"),
            "spire_compose_dir": str(self.root / "compose"),
            "spire_agent_trust_bundle_path": str(self.state / "bootstrap-bundle.pem"),
            "spire_bin_dir": "/opt/spire/bin", "spire_container_uid": str(os.getuid()),
            "spire_log_level": "INFO",
        }
        fake = self.root / "bin/docker"
        fake.write_text('''#!/usr/bin/python3
import json, os, pathlib, sys
args = sys.argv[1:]
with open(os.environ['SPIRE_TEST_CALLS'], 'a') as f:
    f.write(json.dumps(args) + '\\n')
mode = os.environ.get('SPIRE_TEST_MODE', '')
if '-socketPath' in args and args[args.index('-socketPath')+1].endswith('server.sock'):
    sys.exit('server admin socket does not exist')
if 'container' in args:
    kind = 'SERVER' if any('spire-server' in a for a in args) else 'AGENT'
    if os.environ.get('SPIRE_TEST_' + kind + '_PRESENT', '1') == '1': print('fixture-id')
elif 'bundle' in args:
    if mode == 'export-failed': sys.exit('export refused')
    bundle = pathlib.Path(os.environ['SPIRE_TEST_BUNDLE_FILE']).read_text()
    if mode == 'bad-bundle': bundle = 'not a certificate'
    if mode == 'malformed-bundle': bundle = '-----BEGIN CERTIFICATE-----\\ninvalid\\n-----END CERTIFICATE-----'
    if mode == 'partial-bundle': bundle += '\\n-----BEGIN CERTIFICATE-----\\ninvalid'
    print(bundle)
elif 'token' in args:
    if mode == 'token-denied': sys.exit('token generation denied')
    print('Token: ' + os.environ['SPIRE_TEST_TOKEN'])
elif 'healthcheck' in args:
    if mode == 'unhealthy': sys.exit('agent unhealthy')
elif 'compose' in args:
    pathlib.Path(os.environ['SPIRE_TEST_STATE']).write_text('{}')
    print('Started', file=sys.stderr)
elif '-version' in args:
    kind = 'SERVER' if any(a.endswith('/spire-server') or a == 'spire-server' for a in args) else 'AGENT'
    if 'run' in args:
        if mode == 'candidate-failed': sys.exit('candidate version probe failed')
        print(os.environ.get('SPIRE_TEST_CANDIDATE_' + kind + '_VERSION', '1.16.0'), file=sys.stderr)
        sys.exit(0)
    if mode == 'unreadable-version': sys.exit('cannot execute stopped container')
    print(os.environ.get('SPIRE_TEST_' + kind + '_VERSION', '1.16.0'), file=sys.stderr)
else:
    sys.exit('unexpected Docker command')
''')
        fake.chmod(0o755)
        self.env = dict(os.environ, PATH=str(self.root / "bin") + ":" + os.environ["PATH"],
                        SPIRE_TEST_CALLS=str(self.calls), SPIRE_TEST_BUNDLE_FILE=str(cert),
                        SPIRE_TEST_STATE=str(self.state / "agent-data.json"),
                        SPIRE_TEST_TOKEN="fixture-token-redaction-control",
                        ANSIBLE_ROLES_PATH=str(self.root / "roles"),
                        ANSIBLE_LOCAL_TEMP=str(self.root / "ansible-local"),
                        ANSIBLE_HOST_KEY_CHECKING="False", ANSIBLE_NOCOLOR="1")
        inventory = {"all": {"hosts": {
            "ca": {"ansible_connection": "local", "spire_enabled": True},
            "node": {"ansible_connection": "local", "spire_enabled": False},
        }}}
        (self.root / "inventory.yml").write_text(yaml.safe_dump(inventory))

    def run_tasks(self, tasks=None, mode=""):
        if tasks is None:
            tasks = yaml.safe_load((ROLE / "tasks/agent-only.yml").read_text())
        # Adapt privileges only; retain real command/when/register semantics.
        def adapt(items):
            for task in items:
                if "ansible.builtin.package" in task:
                    task["when"] = False  # no package-manager writes on the test runner
                for action in ("ansible.builtin.file", "ansible.builtin.copy", "ansible.builtin.template"):
                    if action in task:
                        for key in ("owner", "group"):
                            task[action].pop(key, None)
                        if "loop" in task:
                            for item in task["loop"]:
                                item["owner"] = str(os.getuid())
                if "retries" in task:
                    task["retries"], task["delay"] = 1, 0
                for key in ("block", "rescue", "always"):
                    adapt(task.get(key, []))
        adapt(tasks)
        task_dir = self.root / "roles/spire/tasks"
        task_dir.mkdir(exist_ok=True)
        (task_dir / "main.yml").write_text(yaml.safe_dump(tasks, sort_keys=False))
        play = [{"hosts": "all", "gather_facts": False, "vars": self.vars,
                 "tasks": [{"ansible.builtin.include_role": {"name": "spire"},
                            "when": "inventory_hostname == 'node'"}]}]
        path = self.root / "play/play.yml"
        path.write_text(yaml.safe_dump(play))
        proc = subprocess.run(["ansible-playbook", "-i", str(self.root / "inventory.yml"), str(path)],
                              env=dict(self.env, SPIRE_TEST_MODE=mode), capture_output=True, text=True,
                              cwd=self.root, timeout=90)
        self.assertNotIn(self.env["SPIRE_TEST_TOKEN"], proc.stdout + proc.stderr,
                         "join token leaked through Ansible output")
        return proc

    def commands(self):
        return [json.loads(line) for line in self.calls.read_text().splitlines()] if self.calls.exists() else []

    def test_first_enrollment_uses_default_socket_and_persists_state(self):
        result = self.run_tasks()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertTrue((self.state / "agent-data.json").exists())
        self.assertEqual(sum("token" in cmd for cmd in self.commands()), 1)
        self.assertTrue(any("healthcheck" in cmd for cmd in self.commands()))

    def test_rerun_does_not_mint_a_token(self):
        (self.state / "agent-data.json").write_text('{}')
        result = self.run_tasks()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertFalse(any("token" in cmd for cmd in self.commands()))

    def test_bad_bundle_or_export_failure_preserves_trust_and_does_not_start(self):
        bundle = self.state / "bootstrap-bundle.pem"
        for mode in ("bad-bundle", "export-failed", "malformed-bundle", "partial-bundle"):
            with self.subTest(mode=mode):
                bundle.write_text("previous-trust")
                self.calls.unlink(missing_ok=True)
                self.assertNotEqual(self.run_tasks(mode=mode).returncode, 0)
                self.assertEqual(bundle.read_text(), "previous-trust")
                self.assertFalse(any("compose" in cmd for cmd in self.commands()))

    def test_token_denial_does_not_start_agent(self):
        self.assertNotEqual(self.run_tasks(mode="token-denied").returncode, 0)
        self.assertFalse(any("compose" in cmd for cmd in self.commands()))

    def test_persisted_file_does_not_hide_unhealthy_agent(self):
        (self.state / "agent-data.json").write_text('{}')
        self.assertNotEqual(self.run_tasks(mode="unhealthy").returncode, 0)

    def test_version_guard_rejects_agent_before_server_and_large_jumps(self):
        tasks = yaml.safe_load((ROLE / "tasks/version-check.yml").read_text())
        cases = [
            (True, "1.11.2", "1.11.2", False),
            (True, "1.16.0", "1.15.5", True),
            (True, "1.15.5", "1.15.5", False),
            (False, "1.11.2", "1.11.2", False),
            (False, "1.15.5", "1.15.5", True),
            (False, "1.16.0", "1.14.9", False),
            (False, "1.17.0", "1.16.0", False),
        ]
        self.vars["spire_server_image"] = "ghcr.io/spiffe/spire-server:1.16.0@sha256:" + "2" * 64
        self.vars["spire_ca_holder"] = "ca"
        for agent_only, server, agent, allowed in cases:
            with self.subTest(agent_only=agent_only, server=server, agent=agent):
                self.vars["spire_agent_only"] = agent_only
                self.env.update(SPIRE_TEST_SERVER_VERSION=server, SPIRE_TEST_AGENT_VERSION=agent)
                result = self.run_tasks(tasks=yaml.safe_load(yaml.safe_dump(tasks)))
                self.assertEqual(result.returncode == 0, allowed, result.stdout + result.stderr)
        self.assertFalse(any("compose" in cmd for cmd in self.commands()))

    def test_version_guard_refuses_unreadable_server(self):
        self.assertNotEqual(self.run_tasks(mode="unreadable-version").returncode, 0)
        self.assertFalse((self.root / "config").exists())

    def test_fresh_install_and_orphaned_state(self):
        tasks = yaml.safe_load((ROLE / "tasks/version-check.yml").read_text())
        self.vars.update(spire_agent_only=False,
                         spire_server_image="ghcr.io/spiffe/spire-server:1.16.0@sha256:" + "2" * 64)
        self.env.update(SPIRE_TEST_SERVER_PRESENT="0", SPIRE_TEST_AGENT_PRESENT="0")
        self.assertEqual(self.run_tasks(tasks=tasks).returncode, 0)
        (self.state / "agent-data.json").write_text('{}')
        self.assertNotEqual(self.run_tasks(tasks=tasks).returncode, 0)
        (self.state / "agent-data.json").unlink()
        server = self.root / "data/server"
        server.mkdir()
        (server / "keys.json").write_text('{}')
        self.assertNotEqual(self.run_tasks(tasks=tasks).returncode, 0)

    def test_ambiguous_or_missing_ca_refused_before_writes(self):
        path = self.root / "inventory.yml"
        for ca_enabled in (False, True):
            hosts = {"ca": {"ansible_connection": "local", "spire_enabled": ca_enabled},
                     "node": {"ansible_connection": "local", "spire_enabled": False}}
            if ca_enabled:
                hosts["second-ca"] = {"ansible_connection": "local", "spire_enabled": True}
            path.write_text(yaml.safe_dump({"all": {"hosts": hosts}}))
            self.assertNotEqual(self.run_tasks().returncode, 0)
            self.assertFalse((self.root / "config").exists())
            self.assertEqual(self.commands(), [])

    def test_agent_only_playbook_does_not_require_local_postgres(self):
        play = yaml.safe_load((REPO / "ops/ansible/spire-deploy.yml").read_text())[0]
        result = self.run_tasks(tasks=play['pre_tasks'])
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(self.commands(), [])

    def test_colocated_first_enrollment_redacts_token(self):
        self.vars.update(spire_db_password="fixture-db-password",
                         spire_server_bind_address="127.0.0.1", spire_server_host_port=8081)
        tasks = yaml.safe_load((ROLE / "tasks/main.yml").read_text())
        start = next(i for i, t in enumerate(tasks) if t.get("register") == "spire_agent_svid")
        end = next(i for i in range(start, len(tasks)) if tasks[i]["name"] == "spire | up spire-agent")
        result = self.run_tasks(tasks[start:end])
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(sum("token" in cmd for cmd in self.commands()), 1)
        self.assertEqual((self.root / "compose/.env.spire").stat().st_mode & 0o777, 0o600)
        self.assertIn("SPIRE_JOIN_TOKEN=" + self.env['SPIRE_TEST_TOKEN'],
                      (self.root / "compose/.env.spire").read_text())

    def test_digest_runtime_version_must_match_advertised_tag(self):
        tasks = yaml.safe_load((ROLE / 'tasks/version-check.yml').read_text())
        self.vars.update(spire_agent_only=False,
                         spire_server_image='ghcr.io/spiffe/spire-server:1.16.0@sha256:' + '2' * 64)
        self.env.update(SPIRE_TEST_SERVER_VERSION='1.15.5', SPIRE_TEST_AGENT_VERSION='1.15.5')
        for kind in ('SERVER', 'AGENT'):
            for value in ('1.11.2', '1.17.0', '', 'not-a-version', '1.16.0\n1.15.5'):
                with self.subTest(kind=kind, value=value):
                    self.env['SPIRE_TEST_CANDIDATE_' + kind + '_VERSION'] = value
                    self.assertNotEqual(self.run_tasks(tasks=tasks).returncode, 0)
                    self.assertFalse((self.root / 'config').exists())
                    self.env.pop('SPIRE_TEST_CANDIDATE_' + kind + '_VERSION')
        self.assertNotEqual(self.run_tasks(tasks=tasks, mode='candidate-failed').returncode, 0)
        self.assertFalse(any('compose' in cmd for cmd in self.commands()))

    def test_digest_only_mirror_reference_uses_binary_version(self):
        self.vars['spire_agent_image'] = 'mirror.example.org:5000/spire-agent@sha256:' + '1' * 64
        self.assertEqual(self.run_tasks().returncode, 0)

    def test_colocated_rerun_does_not_mint_a_token(self):
        (self.state / "agent-data.json").write_text('{}')
        tasks = yaml.safe_load((ROLE / "tasks/main.yml").read_text())
        # Execute the actual contiguous enrollment block, before compose bring-up.
        start = next(i for i, t in enumerate(tasks) if t.get("register") == "spire_agent_svid")
        end = next(i for i in range(start, len(tasks)) if tasks[i]["name"] == "spire | up spire-agent")
        result = self.run_tasks(tasks[start:end])
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertFalse(any("token" in cmd for cmd in self.commands()))


if __name__ == "__main__":
    unittest.main()
