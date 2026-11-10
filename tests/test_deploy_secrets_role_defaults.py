"""A deploy's secrets include the env lookups in its roles' defaults, not only group_vars.

goat-get reads its release token in defaults/main.yml (GOAT_CLI_RELEASE_TOKEN, else
GH_TOKEN, else GHCR_READ_TOKEN); the scan that only read group_vars never asked for
it, and a deploy without it failed on the host. goatbuilder mirrors the same
derivation (crates/goatbuilder/src/doctor.rs, a_role_default_secret_is_required).

Run: python3 -m unittest tests/test_deploy_secrets_role_defaults.py -v
"""
import importlib.util
import os
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


class RoleDefaults(unittest.TestCase):
    def fixture(self, group_vars="site_id: dev-a\n"):
        root = tempfile.mkdtemp()
        write(root, "ops/ansible/goat-get-deploy.yml", '- hosts: "{{ target }}"\n  roles: [goat-get]\n')
        write(root, "ops/ansible/group_vars/dev_hosts/vars.yml", group_vars)
        write(root, "ops/ansible/roles/goat-get/defaults/main.yml", """\
            goat_get_github_token: "{{ lookup('env', 'GOAT_CLI_RELEASE_TOKEN') | default(lookup('env', 'GH_TOKEN'), true) | default(lookup('env', 'GHCR_READ_TOKEN'), true) }}"
            goat_get_cache: "{{ lookup('env', 'HOME') }}/.cache"
            goat_get_optional: "{{ lookup('env', 'OPTIONAL_THING') | default('', true) }}"
            goat_get_nb: "{{ lookup('env', (site_id | mandatory | upper | replace('-', '_')) + '_NB_API_TOKEN') }}"
            goat_get_key: "{{ lookup('env', 'ONE_KEY') }}"  # or: MANY_KEYS
            goat_get_unused: "{{ lookup('env', 'NEVER_READ') }}"
            """)
        write(root, "ops/ansible/roles/goat-get/tasks/main.yml",
              '- debug: msg="{{ goat_get_github_token }} {{ goat_get_cache }} {{ goat_get_optional }} {{ goat_get_nb }} {{ goat_get_key }}"\n')
        return root

    def needed(self, root):
        return load().needed_for(root, os.path.join(root, "ops/ansible/goat-get-deploy.yml"), ["goat-get"], ["dev_hosts"])

    def test_role_default_secrets_are_needed(self):
        got = self.needed(self.fixture())
        self.assertEqual(got["goat_get_github_token"], ["GOAT_CLI_RELEASE_TOKEN", "GH_TOKEN", "GHCR_READ_TOKEN"])
        self.assertEqual(got["goat_get_nb"], ["DEV_A_NB_API_TOKEN"])
        self.assertEqual(got["goat_get_key"], ["ONE_KEY", "MANY_KEYS"])
        for not_a_need in ("goat_get_cache", "goat_get_optional", "goat_get_unused"):
            self.assertNotIn(not_a_need, got)

    def test_a_group_that_sets_the_variable_takes_it_over(self):
        got = self.needed(self.fixture("site_id: dev-a\ngoat_get_github_token: literal\n"))
        self.assertNotIn("goat_get_github_token", got)

    def test_the_real_goat_get_needs_its_release_token_on_zagros_dev(self):
        """With no token anywhere, the driver's gate names it, as any one of three names."""
        env = {"PATH": os.environ["PATH"], "HOME": os.environ.get("HOME", "/tmp"), "GOAT_SECRET_STORE": "env"}
        out = subprocess.run(
            [sys.executable, SCRIPT, "--repo", REPO, "--component", "goat-get",
             "--play", os.path.join(REPO, "ops/ansible/goat-get-deploy.yml"), "zagros_dev_hosts"],
            capture_output=True, text=True, env=env)
        self.assertEqual(out.returncode, 1, out.stdout + out.stderr)
        self.assertIn("goat_get_github_token GOAT_CLI_RELEASE_TOKEN|GH_TOKEN|GHCR_READ_TOKEN MISSING", out.stdout)
        env["GH_TOKEN"] = "x"
        out = subprocess.run(
            [sys.executable, SCRIPT, "--repo", REPO, "--component", "goat-get",
             "--play", os.path.join(REPO, "ops/ansible/goat-get-deploy.yml"), "zagros_dev_hosts"],
            capture_output=True, text=True, env=env)
        self.assertIn("goat_get_github_token GOAT_CLI_RELEASE_TOKEN|GH_TOKEN|GHCR_READ_TOKEN present", out.stdout)

    def test_the_hint_names_the_alternatives_and_the_gh_fallback(self):
        spec = importlib.util.spec_from_file_location("posture", os.path.join(REPO, "ops/secrets/posture.py"))
        posture = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(posture)
        lines = " ".join(posture.hint(["GOAT_CLI_RELEASE_TOKEN|GH_TOKEN|GHCR_READ_TOKEN"], REPO))
        self.assertIn("any one of GOAT_CLI_RELEASE_TOKEN, GH_TOKEN, GHCR_READ_TOKEN", lines)
        self.assertIn("GH_TOKEN=$(gh auth token)", lines)



class IncludedStoreRole(unittest.TestCase):
    """A store role the component includes holds its database password, named through
    a variable and read only on Postgres: the check follows include_role, resolves the
    name through the variable, and asks for it only where the store is postgres."""

    def fixture(self, store):
        root = tempfile.mkdtemp()
        write(root, "ops/ansible/app-deploy.yml", '- hosts: "{{ target }}"\n  roles: [app]\n')
        write(root, "ops/ansible/group_vars/dev_hosts/vars.yml", f"site_id: dev-a\napp_store: {store}\n")
        write(root, "ops/ansible/roles/app/tasks/main.yml", """\
            - name: app | the store
              ansible.builtin.include_role:
                name: app-store
                public: true
            """)
        write(root, "ops/ansible/roles/app-store/defaults/main.yml", """\
            app_store: sqlite
            app_db_password_env: "{{ site_id | upper | replace('-', '_') }}_APP_DB_PASS"
            app_db_password: "{{ lookup('env', app_db_password_env) }}"  # when: app_store=postgres
            """)
        write(root, "ops/ansible/roles/app-store/tasks/main.yml", '- debug: msg="{{ app_db_password | length }}"\n')
        return root

    def needed(self, root):
        return load().needed_for(root, os.path.join(root, "ops/ansible/app-deploy.yml"), ["app"], ["dev_hosts"])

    def test_postgres_goatnet_needs_the_included_roles_password(self):
        self.assertEqual(self.needed(self.fixture("postgres")).get("app_db_password"), ["DEV_A_APP_DB_PASS"])

    def test_sqlite_goatnet_does_not(self):
        self.assertNotIn("app_db_password", self.needed(self.fixture("sqlite")))

    def test_closure_follows_include_role(self):
        self.assertEqual(load().role_closure(self.fixture("postgres"), ["app"]), ["app", "app-store"])

if __name__ == "__main__":
    unittest.main()
