"""A deploy's secrets include the env lookups its tasks make, not only vars and defaults.

audit-subscriber resolves its S3 keys with set_fact and asserts them, and resolves the
MinIO root pair in an included lifecycle.yml that runs after the container has been
recreated. A pre-check that read only group_vars and role defaults named none of the
four, so a deploy without them replaced the container and stopped half-way. The check
walks the tasks the play runs (includes, included roles, the roles the play lists),
carries each `when:` down, and names every lookup a reached task makes.

Run: python3 -m unittest tests/test_deploy_secrets_task_lookups.py -v
"""
import importlib.util
import os
import shutil
import subprocess
import sys
import tempfile
import textwrap
import unittest

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SCRIPT = os.path.join(REPO, "ops/scripts/deploy-secrets-needed.py")


def load():
    spec = importlib.util.spec_from_file_location("deploy_secrets_needed", SCRIPT)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def write(root, rel, text):
    path = os.path.join(root, rel)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as fh:
        fh.write(textwrap.dedent(text))


class TaskLookups(unittest.TestCase):
    def fixture(self, group_vars):
        root = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, root, True)
        write(root, "ops/ansible/app-deploy.yml", """\
            - hosts: "{{ target | mandatory }}"
              pre_tasks:
                - ansible.builtin.include_tasks: tasks/guard.yml
              roles:
                - role: app
                - role: app-client
            """)
        write(root, "ops/ansible/tasks/guard.yml", "- ansible.builtin.debug: msg=guard\n")
        write(root, "ops/ansible/group_vars/dev_hosts/vars.yml", group_vars)
        write(root, "ops/ansible/roles/app/defaults/main.yml", """\
            app_s3_endpoint: ""
            app_apply_policy: true
            app_s3_key_envvar: "{{ site_id | upper | replace('-', '_') }}_APP_S3_KEY"
            app_root_user_envvar: "{{ store_root_user_envvar | default('') }}"
            app_token_envvar: "APP_OPTIONAL_TOKEN"
            """)
        write(root, "ops/ansible/roles/app/tasks/main.yml", """\
            - name: resolve the S3 key when the endpoint is set
              ansible.builtin.set_fact:
                app_s3_key: "{{ lookup('env', app_s3_key_envvar) }}"
              when: app_s3_endpoint | length > 0
            - name: an optional token, guarded by its own presence
              ansible.builtin.copy:
                content: "{{ lookup('env', app_token_envvar) }}"
                dest: /tmp/x
              when: lookup('env', app_token_envvar) | length > 0
            - name: a soft read
              ansible.builtin.set_fact:
                app_soft: "{{ lookup('env', 'APP_SOFT') | default('', true) }}"
            - name: recreate the container
              ansible.builtin.debug: msg=recreate
            - name: later phase
              ansible.builtin.include_tasks:
                file: policy.yml
              when: app_apply_policy | bool
            """)
        write(root, "ops/ansible/roles/app/tasks/policy.yml", """\
            - block:
                - name: resolve root credentials
                  ansible.builtin.set_fact:
                    app_root_user: "{{ lookup('env', app_root_user_envvar) }}"
                - name: the shell's own name is no secret
                  ansible.builtin.set_fact:
                    app_who: "{{ lookup('env', 'USER') }}"
            """)
        write(root, "ops/ansible/roles/app-client/tasks/main.yml", """\
            - ansible.builtin.assert:
                that: (lookup('env', 'APP_CLIENT_SECRET') | length) >= 16
            """)
        return root

    def needed(self, root):
        return load().needed_for(root, os.path.join(root, "ops/ansible/app-deploy.yml"), ["app"], ["dev_hosts"])

    def test_every_lookup_a_reached_task_makes_is_needed(self):
        got = self.needed(self.fixture(
            "site_id: zagros\napp_s3_endpoint: http://store:9000\nstore_root_user_envvar: ZAGROS_STORE_ROOT_USER\n"))
        self.assertEqual(got.get("app_s3_key"), ["ZAGROS_APP_S3_KEY"])
        self.assertEqual(got.get("app_root_user"), ["ZAGROS_STORE_ROOT_USER"], "a later phase's lookup, through an include")
        self.assertEqual(got.get("app-client:APP_CLIENT_SECRET"), ["APP_CLIENT_SECRET"], "a role the play lists")
        for v, alts in got.items():
            self.assertNotIn("APP_OPTIONAL_TOKEN", alts, "a task guarded by its own env lookup is optional")
            self.assertNotIn("APP_SOFT", alts, "a literal fallback makes it optional")
            self.assertNotIn("USER", alts)

    def test_a_condition_that_is_false_on_the_goatnet_drops_the_need(self):
        got = self.needed(self.fixture("site_id: zagros\napp_apply_policy: false\n"))
        self.assertNotIn("app_s3_key", got, "observe-only: no endpoint, no S3 key")
        self.assertNotIn("app_root_user", got, "the policy phase is off")

    def test_eval_when_shapes(self):
        ev = load().eval_when
        s, r = {"a": "x", "e": "", "f": "false"}, {"n": "3"}
        self.assertTrue(ev("a | length > 0", s, r))
        self.assertFalse(ev("e | length > 0", s, r))
        self.assertFalse(ev("f | bool", s, r))
        self.assertTrue(ev("not f | bool", s, r))
        self.assertFalse(ev("a == 'y'", s, r))
        self.assertIsNone(ev("unknown | bool", s, r))
        self.assertIsNone(ev("a or e", s, r))


class RealAuditSubscriber(unittest.TestCase):
    """The real audit-subscriber play and role on a synthetic goatnet in durable-write
    mode: the pre-check names the S3 pair and the MinIO root pair before any host."""

    def setUp(self):
        self.root = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.root, True)
        a = os.path.join(self.root, "ops/ansible")
        os.makedirs(os.path.join(a, "roles"))
        shutil.copy(os.path.join(REPO, "ops/ansible/audit-subscriber-deploy.yml"), a)
        shutil.copytree(os.path.join(REPO, "ops/ansible/tasks"), os.path.join(a, "tasks"))
        for r in ("audit-subscriber", "rollback-ledger"):
            shutil.copytree(os.path.join(REPO, "ops/ansible/roles", r), os.path.join(a, "roles", r))
        write(self.root, "ops/ansible/group_vars/zagros_hosts/audit-subscriber.yml", """\
            site_id: zagros
            audit_subscriber_s3_endpoint: "http://127.0.0.1:9000"
            minio_root_user_envvar: "ZAGROS_MINIO_ROOT_USER"
            minio_root_password_envvar: "ZAGROS_MINIO_ROOT_PASSWORD"
            """)

    def run_check(self, extra_env):
        env = {"PATH": os.environ["PATH"], "HOME": os.environ.get("HOME", "/tmp"), "GOAT_SECRET_STORE": "env", **extra_env}
        return subprocess.run(
            [sys.executable, SCRIPT, "--repo", self.root, "--component", "audit-subscriber",
             "--play", os.path.join(self.root, "ops/ansible/audit-subscriber-deploy.yml"), "zagros_hosts"],
            capture_output=True, text=True, env=env)

    WANT = ("ZAGROS_AUDIT_SUBSCRIBER_S3_ACCESS_KEY", "ZAGROS_AUDIT_SUBSCRIBER_S3_SECRET_KEY",
            "ZAGROS_MINIO_ROOT_USER", "ZAGROS_MINIO_ROOT_PASSWORD")

    def test_all_four_are_named_and_missing_refuses(self):
        out = self.run_check({})
        self.assertEqual(out.returncode, 1, out.stdout + out.stderr)
        for name in self.WANT:
            self.assertIn(f" {name} MISSING", out.stdout)

    def test_all_present_passes(self):
        out = self.run_check({n: "x" for n in self.WANT})
        self.assertEqual(out.returncode, 0, out.stdout + out.stderr)


if __name__ == "__main__":
    unittest.main()
