"""Tasks that handle secrets don't log them, and pass them to containers through
files or stdin, not the command line.

Each case runs real tasks from a playbook or role on localhost at -vvv --diff,
with a stand-in `docker` and `aws` on PATH and a synthetic value for every
secret. A case passes when no secret value, nor its first five characters,
appears in ansible's output, none appears on the command line a stand-in was
given, each secret a case names reached the container or the AWS CLI through a
mode-0600 file or stdin, and no such file is left behind.

A named task is run inside the nearest block that carries an `always:` section
(the cleanup that removes its env file), or alone when there is none. Role
files that provision a database are run whole. Nothing here runs from
ops/ansible, so no checkout ansible.cfg applies, and become is off.
"""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

import yaml

ROOT = Path(__file__).resolve().parents[1]
ANSIBLE = ROOT / 'ops/ansible'

# Each value starts with five characters found nowhere else in a run, so a
# task that prints any five of them at the start is caught.
SECRETS = {
    'postgres_superuser_password': 'Kp7Zq-pgSuperSynthetic',
    'spire_db_password': 'Vb3Wm-spireDbSynthetic',
    'portal_db_password': 'Hx9Lr-portalDbSynthetic',
    'id_admin_console_db_password': 'Jd4Tn-consoleDbSynthetic',
    'cloudflare_api_token': 'Wc8Fy-cfTokenSynthetic',
    'nb_relay_auth_secret': 'Rm2Gs-relayAuthSynthetic',
    'shared_turn_secret': 'Ty6Nd-turnSharedSynthetic',
    'rathole_auth_token': 'Pz5Hk-ratholeTokSynthetic',
    'admit_audit_password': 'Ls1Qv-admitAudSynthetic',
    'operator_idp_pg_app_password': 'Gn3Xb-opIdpAppSynthetic',
    'postgres_replicator_password': 'Fw7Cj-replicaSynthetic',
    'cgnat_site_a_setup_key': 'qz4uv9f3-1a2b-4c3d-9e8f-0a1b2c3d4e5f',
}

# Values the stand-ins print that are themselves secrets.
JOIN_TOKEN = 'Ud4Ye-joinTokSynthetic'

FAKE_DOCKER = r'''#!/usr/bin/env bash
# Stand-in docker: records its command line, every env file it is given (path,
# mode, contents) and what it reads on stdin with -i; answers the queries the
# tasks under test make.
printf '%s\n' "$*" >> "$FAKE_LOG/argv"
envfiles=()
interactive=0
prev=''
for a in "$@"; do
  if [ "$prev" = '--env-file' ]; then envfiles+=("$a"); fi
  case "$a" in
    --env-file=*) envfiles+=("${a#--env-file=}") ;;
    -i|--interactive) interactive=1 ;;
  esac
  prev="$a"
done
for f in ${envfiles[@]+"${envfiles[@]}"}; do
  printf '%s %s\n' "$f" "$(stat -c %a "$f")" >> "$FAKE_LOG/envfiles"
  cat "$f" >> "$FAKE_LOG/env"
done
input=''
if [ "$interactive" = 1 ]; then
  input="$(cat)"
  printf '%s\n' "$input" >> "$FAKE_LOG/stdin"
fi
all="$* $input"
case "$all" in
  *'token generate'*) echo 'Token: Ud4Ye-joinTokSynthetic' ;;
  *pg_is_in_recovery*) echo f ;;
  *"grep -q 'host all admit_audit'"*|*'grep -q host all admit_audit'*) exit 1 ;;
  *pg_get_userbyid*) echo '' ;;
  *'SHOW hba_file'*) echo /var/lib/postgresql/data/pg_hba.conf ;;
  *pg_hba_file_rules*) echo 1 ;;
  *pg_extension*) echo 0 ;;
  *inspect*) echo 'mgmt ' ;;
  *pg_roles*) echo '' ;;
  *'FROM pg_database WHERE datname'*) echo '' ;;
  *pg_stat_archiver*) echo 5 ;;
  *'FROM peers'*) echo 50 ;;
  *'SELECT 1'*) echo 1 ;;
esac
exit 0
'''

FAKE_AWS = r'''#!/usr/bin/env bash
# Stand-in aws: records its command line, and the path, mode and contents of
# every file:// parameter it is given.
printf '%s\n' "$*" >> "$FAKE_LOG/argv"
for a in "$@"; do
  case "$a" in
    file://*)
      f="${a#file://}"
      printf '%s %s\n' "$f" "$(stat -c %a "$f")" >> "$FAKE_LOG/envfiles"
      cat "$f" >> "$FAKE_LOG/env" ;;
  esac
done
exit 0
'''


def require_ansible_playbook(case):
    """Skip without ansible-playbook, unless GOAT_TEST_REQUIRE_ANSIBLE=1 (CI sets
    it), where its absence fails the test instead of skipping it quietly."""
    if shutil.which('ansible-playbook'):
        return
    if os.environ.get('GOAT_TEST_REQUIRE_ANSIBLE') == '1':
        case.fail('GOAT_TEST_REQUIRE_ANSIBLE=1 but ansible-playbook is not on PATH')
    case.skipTest('ansible-playbook is required')


class _Loader(yaml.SafeLoader):
    pass


_Loader.add_constructor('!unsafe', lambda loader, node: loader.construct_scalar(node))


def task_lists(doc):
    """Every task list in a playbook or a task file, outermost first."""
    if not isinstance(doc, list):
        return []
    if any(isinstance(p, dict) and ('hosts' in p or 'import_playbook' in p) for p in doc):
        out = []
        for play in doc:
            for key in ('pre_tasks', 'tasks', 'post_tasks'):
                out.append(play.get(key) or [])
        return out
    return [doc]


def find(tasks, name, wrappers=()):
    """The task named `name` inside the nearest enclosing block with `always:`
    (or the task alone), or None."""
    for t in tasks:
        if not isinstance(t, dict):
            continue
        if t.get('name') == name:
            return wrappers[-1] if wrappers else t
        inner = wrappers + ((t,) if 'always' in t else ())
        for key in ('block', 'rescue', 'always'):
            hit = find(t.get(key) or [], name, inner)
            if hit is not None:
                return hit
    return None


def extract(path, names):
    doc = yaml.load(path.read_text(), Loader=_Loader)
    picked = []
    for name in names:
        hit = None
        for tasks in task_lists(doc):
            hit = find(tasks, name)
            if hit is not None:
                break
        if hit is None:
            raise AssertionError(f'{path}: no task named {name!r}')
        if not any(hit is p for p in picked):
            picked.append(hit)
    return picked


def redirect_etc(node, tmp):
    """A copy into /etc or /tmp writes into the test's directory instead,
    owned by whoever runs the test."""
    if isinstance(node, list):
        for n in node:
            redirect_etc(n, tmp)
    elif isinstance(node, dict):
        for key in ('copy', 'ansible.builtin.copy'):
            args = node.get(key)
            if isinstance(args, dict) and str(args.get('dest', '')).startswith(('/etc/', '/tmp/')):
                args['dest'] = str(tmp / Path(args['dest']).name)
                args.pop('owner', None)
                args.pop('group', None)
        for v in node.values():
            redirect_etc(v, tmp)


# (label, file, task names or None for the whole file, extra vars,
#  secrets that must reach a container through an env file or stdin)
CASES = [
    ('spire datastore role and database', 'roles/spire/tasks/main.yml',
     ['spire | check spire db role exists', 'spire | create spire db role',
      "spire | reconcile the spire db role's password",
      'spire | check spire database exists', 'spire | create spire database'],
     {'spire_db_user': 'spire', 'spire_db_name': 'spire'},
     ['postgres_superuser_password', 'spire_db_password']),
    ('portal registry database', 'roles/goat-zenoh-portal-store/tasks/postgres-db.yml', None,
     {'portal_db_container': 'postgres-a', 'portal_db_user': 'goat_portal',
      'portal_db_name': 'goat_portal', 'portal_db_host': 'postgres-a',
      'portal_db_password_env': 'ZAGROS_PORTAL_DB_PASS', 'portal_traefik_network': 'mgmt'},
     ['postgres_superuser_password', 'portal_db_password']),
    ('id console databases', 'roles/id-admin-console-store/tasks/postgres-db.yml', None,
     {'id_admin_console_db_container': 'postgres-a', 'id_admin_console_db_user': 'id_console',
      'id_admin_console_onboard_db_name': 'id_onboard', 'id_admin_console_org_db_name': 'id_org',
      'id_admin_console_db_password_env': 'ZAGROS_ID_CONSOLE_DB_PASS'},
     ['postgres_superuser_password', 'id_admin_console_db_password']),
    ('cgnat setup-key hash and seed SQL', 'cgnat-validation.yml',
     ['Compute setup-key hash + mask', 'Render seed SQL'],
     {'site_uuid_upper': "{{ cgnat_site_a_setup_key | upper }}", 'site_account_id': 'acc_a',
      'site_key_id': 'sk_a', 'site_group_id': 'grp_a', 'site_domain': 'cgnat-a.local',
      'site_net_id': 'cgnat-a-net', 'site_network_net': '{"IP":"100.64.0.0","Mask":"//8AAA=="}'},
     [], 'cgnat_seed'),
    ('cgnat seed and peer counts', 'cgnat-validation.yml',
     ['Apply seed SQL via psql', 'Query enrolled peer count (site α)',
      'Query enrolled peer count (site β)'], {}, ['postgres_superuser_password']),
    ('relay DNS-01 credentials', 'goat-relay-deploy.yml',
     ['LE — write Cloudflare DNS-01 credentials (0600)'], {'goat_relay_le_dns01': True}, []),
    ('mgmt-host password round trip', 'mgmt-host.yml',
     ['F-044 preflight — postgres password auth round-trip'], {}, ['postgres_superuser_password']),
    ('pgbackrest archiver counts', 'pgbackrest-wave18.yml',
     ['Check pg_stat_archiver on A', 'Check pg_stat_archiver on B (archive_mode=always)'], {},
     ['postgres_superuser_password']),
    ('netbird relay credentials and container', 'relay-server.yml',
     ['Write Cloudflare DNS-01 credentials (0600)',
      'Deploy Netbird relay container (with TLS cert from certbot)'],
     {'relay_cert_dns01': True, 'relay_dns_name': 'relay.zagros.example.org'},
     ['nb_relay_auth_secret']),
    ('admit-audit role, password and grants', 'roles/admit-audit/tasks/pg-role.yml', None,
     {'pg_admit_audit_role': 'admit_audit', 'pg_admit_audit_db': 'netbird'},
     ['postgres_superuser_password', 'admit_audit_password']),
    ('admit-audit pg_hba reload and env', 'roles/admit-audit/tasks/deploy.yml',
     ['Ensure pg_hba.conf admits admit_audit from the docker compose network',
      'Append admit-audit env vars to /opt/mgmt-host/.env'],
     {'admit_audit_image': 'goat-admit-audit:test'}, []),
    ('operator-idp app user smoke', 'roles/operator-idp-postgres/tasks/main.yml',
     ['Smoke — verify operator_idp app user can connect'],
     {'operator_idp_pg_container_name': 'operator-idp-postgres',
      'operator_idp_pg_app_user': 'operator_idp'}, ['operator_idp_pg_app_password']),
    ('streaming replicator role and base backup',
     'roles/postgres-streaming-replication/tasks/main.yml',
     ['Primary | ensure replicator role exists with provided password',
      'Standby | run pg_basebackup A→B with -R (writes standby.signal + primary_conninfo)'],
     {'router_role': 'A', 'postgres_streaming_container': 'postgres-a',
      'postgres_data_volume_path': '/srv/pgdata', 'postgres_basebackup_client_image': 'postgres:16',
      'postgres_primary_lan_ip': '192.0.2.20', 'postgres_primary_port': 5432},
     ['postgres_replicator_password']),
    ('relay TURN secret', 'relay-server.yml',
     ['Read existing TURN secret', 'Set TURN secret fact'], {}, []),
    ('relay credentials in Secrets Manager', 'relay-server.yml',
     ['Store rathole credentials in Secrets Manager'],
     {'aws_cmd': '', 'aws_region': 'eu-west-1', 'rathole_secret_name': 'zagros/rathole',
      'rathole_public_key': 'ratholePublicKeyNotSecret', 'vps_public_ip': '192.0.2.10',
      'turn_secret': 'Ty6Nd-turnSharedSynthetic'},
     ['rathole_auth_token', 'shared_turn_secret']),
    ('relay status', 'relay-server.yml', ['Print relay status'],
     {'vps_public_ip': '192.0.2.10', 'rathole_public_key': 'ratholePublicKeyNotSecret',
      'relay_dns_name': 'relay.zagros.example.org'}, []),
    ('spire join token', 'roles/spire/tasks/main.yml',
     ['spire | generate one-time join token (only when agent not yet attested)',
      'spire | parse token'],
     {'spire_bin_dir': '/opt/spire/bin', 'spire_trust_domain': 'zagros.example.org',
      'spire_agent_node_path': 'node/a', 'spire_agent_svid': {'stat': {'exists': False}}},
     []),
    ('site bootstrap env and password round trip', 'site-bootstrap.yml',
     ['render minimal .env for postgres-spock-only',
      'F-044 preflight — postgres password auth round-trip'],
     {'site_id': 'zagros', 'ansible_date_time': {'iso8601': '2026-01-01T00:00:00Z'}},
     ['postgres_superuser_password']),
]


class SecretTasksDoNotLogOrPassOnTheCommandLine(unittest.TestCase):

    def run_case(self, label, rel, names, extra, delivered, post=None):
        require_ansible_playbook(self)
        path = ANSIBLE / rel
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            log = tmp / 'log'
            bindir = tmp / 'bin'
            log.mkdir()
            bindir.mkdir()
            for tool, text in (('docker', FAKE_DOCKER), ('aws', FAKE_AWS)):
                (bindir / tool).write_text(text)
                (bindir / tool).chmod(0o755)
            if names is None:
                tasks = [{'name': f'run {rel}',
                          'ansible.builtin.include_tasks': str(path)}]
            else:
                tasks = extract(path, names)
                redirect_etc(tasks, tmp)
            play = [{
                'name': label,
                'hosts': 'localhost',
                'connection': 'local',
                'gather_facts': False,
                'vars': dict(SECRETS, compose_dir=str(tmp / 'compose'), **extra),
                'tasks': tasks,
            }]
            (tmp / 'compose').mkdir()
            (tmp / 'compose' / '.env').write_text('')
            (tmp / 'play.yml').write_text(yaml.safe_dump(play, allow_unicode=True, sort_keys=False))
            env = dict(os.environ,
                       PATH=f'{bindir}:{os.environ["PATH"]}',
                       FAKE_LOG=str(log),
                       ANSIBLE_BECOME='False',
                       ANSIBLE_NOCOLOR='1',
                       ANSIBLE_RETRY_FILES_ENABLED='False',
                       ANSIBLE_LOCALHOST_WARNING='False',
                       ANSIBLE_INVENTORY_UNPARSED_WARNING='False')
            env.pop('ANSIBLE_CONFIG', None)
            r = subprocess.run(
                ['ansible-playbook', '-vvv', '--diff', '-i', 'localhost,', 'play.yml'],
                cwd=tmp, env=env, capture_output=True, text=True, timeout=600)
            out = r.stdout + r.stderr
            self.assertEqual(r.returncode, 0, f'{label}: ansible-playbook failed\n{out[-6000:]}')

            def read(name):
                p = log / name
                return p.read_text() if p.exists() else ''

            def lines_with(text, value):
                return '\n'.join(l for l in text.splitlines() if value in l)[:3000]

            argv = read('argv')
            values = dict(SECRETS, spire_join_token=JOIN_TOKEN)
            values['cgnat_site_a_setup_key, upper case'] = SECRETS['cgnat_site_a_setup_key'].upper()
            for var, value in values.items():
                # Its first five characters too: no task prints any part of a
                # secret, a prefix or a mask included.
                self.assertFalse(value[:5] in out,
                                 f'{label}: {var} (its first 5 characters) appears in the -vvv '
                                 f'output:\n{lines_with(out, value[:5])}')
                self.assertFalse(value in out,
                                 f'{label}: {var} appears in the -vvv output:\n{lines_with(out, value)}')
                self.assertFalse(value in argv,
                                 f'{label}: {var} is on a command line:\n{lines_with(argv, value)}')
            if delivered:
                self.assertTrue(argv, f'{label}: no stand-in was run\n{out[-3000:]}')
                received = read('env') + read('stdin')
                for var in delivered:
                    self.assertIn(SECRETS[var], received,
                                  f'{label}: {var} never reached its consumer\n{argv}')
                files = read('envfiles').splitlines()
                self.assertTrue(files or read('stdin'), f'{label}: no file or stdin carried a secret')
                for line in files:
                    f, mode = line.rsplit(' ', 1)
                    self.assertEqual(mode, '600', f'{label}: {f} is mode {mode}')
                    self.assertFalse(Path(f).exists(), f'{label}: {f} was left behind')
            if post == 'cgnat_seed':
                # The seed still carries what NetBird stores: the key's hash and
                # its own masked form, in a file only its owner reads.
                import base64, hashlib
                key = SECRETS['cgnat_site_a_setup_key'].upper()
                seed = tmp / 'cgnat-a-seed.sql'
                text = seed.read_text()
                self.assertIn(base64.b64encode(hashlib.sha256(key.encode()).digest()).decode(), text)
                self.assertIn(f"'{key[:5]}****'", text)
                self.assertEqual(oct(seed.stat().st_mode & 0o777), '0o600', f'{label}: {seed} mode')

    def test_cases(self):
        for label, rel, names, extra, delivered, *post in CASES:
            with self.subTest(label):
                self.run_case(label, rel, names, extra, delivered, *post)


class PreflightAdvisesReadingSecretsFromTheEnvironment(unittest.TestCase):
    """site-preflight's summary prints a site-bootstrap line whose secrets the
    play reads from its own environment, so no value is on that command line."""

    def test_summary(self):
        require_ansible_playbook(self)
        tasks = extract(ANSIBLE / 'site-preflight.yml', ['site-preflight | summary'])
        for profile, names in (('full', ('PG_PASS', 'NB_ENCKEY', 'BKP_KEY')),
                               ('postgres-spock-only', ('PG_PASS',))):
            with self.subTest(profile), tempfile.TemporaryDirectory() as d:
                play = [{'hosts': 'localhost', 'connection': 'local', 'gather_facts': False,
                         'vars': {'site_id': 'zagros', 'site_id_upper': 'ZAGROS',
                                  'mgmt_stack_profile': profile,
                                  'effective_required_env_vars': []},
                         'tasks': tasks}]
                Path(d, 'play.yml').write_text(yaml.safe_dump(play, allow_unicode=True))
                env = dict(os.environ, ANSIBLE_BECOME='False', ANSIBLE_NOCOLOR='1',
                           ANSIBLE_LOCALHOST_WARNING='False',
                           ANSIBLE_INVENTORY_UNPARSED_WARNING='False')
                env.pop('ANSIBLE_CONFIG', None)
                r = subprocess.run(['ansible-playbook', '-i', 'localhost,', 'play.yml'],
                                   cwd=d, env=env, capture_output=True, text=True, timeout=300)
                self.assertEqual(r.returncode, 0, r.stdout[-3000:] + r.stderr[-3000:])
                for name in names:
                    self.assertIn(f'lookup(\\"env\\", \\"ZAGROS_{name}\\")', r.stdout)
                self.assertNotIn('${ZAGROS_', r.stdout)


if __name__ == '__main__':
    unittest.main()
