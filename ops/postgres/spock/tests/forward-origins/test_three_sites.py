#!/usr/bin/env python3
"""A subscription with forward_origins set in a mesh replicates in a loop: the install refuses it.

Three postgres-spock sites on a private Docker network (no host ports) are wired into a full
mesh of six subscriptions, as spock-mesh-peering wires three sites. In a mesh of three or more
sites, a subscription created with forward_origins := '{all}' passes each change on to the
next site, which passes it back, without end, while every subscription still reads
`replicating`. This test writes no rows, so no loop runs; it checks that the install refuses
the setting and that the metric the alert reads reports it.

  1. forward_origins = '{all}' on every subscription: the spock-forward-origins-check role,
     run by ansible-playbook, refuses (GB-351) and names every one of the six subscriptions.
     Its script writes spock_subscription_forward_origins 1 for each subscription at a site.
  2. A group_vars setting spock_mesh_forward_origins: [all] is refused before any catalog read;
     [] and [none] pass.
  3. The same mesh with forward_origins empty: the role passes and the metric is 0.

Substrate: Linux Docker only (CI). Needs ansible-playbook on PATH.

  IMAGE=postgres-spock:smoke python3 test_three_sites.py
  IMAGE=... PFX=fwdr python3 test_three_sites.py   # containers fwdr-a/b/c, network fwdr-net
"""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import uuid

ROOT = Path(__file__).resolve().parents[5]
ROLE = ROOT / 'ops/ansible/roles/spock-forward-origins-check'
SCRIPT = ROLE / 'files/spock-forward-origins.sh'
IMAGE = os.environ.get('IMAGE', '')
PFX = os.environ.get('PFX') or 'forward-origins-' + uuid.uuid4().hex[:8]
NET = PFX + '-net'
PW = 'forward-origins-fixture'
DB = 'netbird'
SITES = ['a', 'b', 'c']


def docker(*args, timeout=120, **kwargs):
    return subprocess.run(['docker', *args], text=True, capture_output=True, timeout=timeout, **kwargs)


def name(site):
    return f'{PFX}-{site}'


def sql(site, statement, db=DB):
    result = docker('exec', name(site), 'psql', '-X', '-U', 'postgres', '-d', db,
                    '-v', 'ON_ERROR_STOP=1', '-tAXq', '-c', statement)
    assert result.returncode == 0, f'{name(site)}: {result.stdout}{result.stderr}'
    return result.stdout.strip()


def up():
    assert docker('network', 'create', NET).returncode == 0, 'network create failed'
    for site in SITES:
        result = docker('run', '-d', '--name', name(site), '--network', NET,
                        '-e', f'POSTGRES_PASSWORD={PW}', IMAGE,
                        'postgres', '-c', 'config_file=/etc/postgresql/postgresql.conf',
                        '-c', f'cluster_name=site_{site}')
        assert result.returncode == 0, result.stderr
    for site in SITES:
        for _ in range(90):
            ok = docker('exec', name(site), 'pg_isready', '-h', '127.0.0.1', '-p', '5432',
                        '-U', 'postgres').returncode == 0
            if ok:
                break
            time.sleep(2)
        else:
            raise AssertionError(f'{name(site)} never accepted a connection')
    # The entrypoint's temporary server can answer once and restart; require three answers.
    for site in SITES:
        good = 0
        for _ in range(60):
            if docker('exec', name(site), 'psql', '-U', 'postgres', '-d', DB, '-tAc',
                      'SELECT 1').returncode == 0:
                good += 1
                if good >= 3:
                    break
            else:
                good = 0
            time.sleep(2)
        assert good >= 3, f'{name(site)} never reached a steady state'
    for site in SITES:
        sql(site, f"CREATE ROLE replicator WITH LOGIN SUPERUSER PASSWORD '{PW}';", db='postgres')
        result = docker('exec', '--user', 'root', name(site), 'bash', '-c',
                        "echo 'host all replicator 0.0.0.0/0 scram-sha-256' >> /etc/postgresql/pg_hba.conf")
        assert result.returncode == 0, result.stderr
        sql(site, 'SELECT pg_reload_conf();', db='postgres')
        sql(site, 'CREATE EXTENSION IF NOT EXISTS spock;')
        sql(site, f"SELECT spock.node_create('site_{site}', "
                  f"'host={name(site)} dbname={DB} user=replicator password={PW}');")


def subscriptions():
    return [(provider, subscriber) for subscriber in SITES for provider in SITES if provider != subscriber]


def wire(forward_origins):
    """A full mesh: on each site, one subscription pulling from each other site."""
    for provider, subscriber in subscriptions():
        sql(subscriber, f"""SELECT spock.sub_create(
            subscription_name := 'sub_site_{provider}_to_site_{subscriber}',
            provider_dsn      := 'host={name(provider)} dbname={DB} user=replicator password={PW}',
            replication_sets  := ARRAY['default']::text[],
            synchronize_data  := false,
            forward_origins   := {forward_origins});""")
    for provider, subscriber in subscriptions():
        sub = f'sub_site_{provider}_to_site_{subscriber}'
        status = ''
        for _ in range(60):
            status = sql(subscriber, f"SELECT status FROM spock.sub_show_status() WHERE subscription_name = '{sub}';")
            if status == 'replicating':
                break
            time.sleep(1)
        assert status == 'replicating', f'{sub} is {status!r}'


def unwire():
    for provider, subscriber in subscriptions():
        sql(subscriber, f"SELECT spock.sub_drop('sub_site_{provider}_to_site_{subscriber}');")


def write_inventory(tmp):
    hosts = {f'fixture_{site}': {
        'ansible_connection': 'local',
        'ansible_python_interpreter': sys.executable,
        'participates_in_spock': True,
        # One goatnet of three sites, each replicating with the other two: the
        # check reads only the peers a site replicates with.
        'site_id': f'site-{site}',
        'inter_site_spock_topology': 'explicit',
        'inter_site_spock_peers': [f'site-{other}' for other in SITES if other != site],
        'spock_mesh_postgres_container': name(site),
    } for site in SITES}
    inventory = tmp / 'inventory.json'
    inventory.write_text(json.dumps({'all': {'children': {'mgmt_hosts': {'hosts': hosts}}}}))
    play = tmp / 'play.json'
    play.write_text(json.dumps([{'hosts': 'fixture_a', 'gather_facts': False,
                                 'roles': ['spock-forward-origins-check']}]))
    config = tmp / 'ansible.cfg'
    config.write_text(f'[defaults]\nroles_path={ROOT}/ops/ansible/roles\nlocal_tmp={tmp}/local\n'
                      f'remote_tmp={tmp}/remote\nretry_files_enabled=False\n')
    return inventory, play, config


def run_role(tmp, *extra):
    inventory, play, config = write_inventory(tmp)
    env = {key: os.environ[key] for key in ('PATH', 'HOME', 'TMPDIR') if key in os.environ}
    env.update(ANSIBLE_CONFIG=str(config), ANSIBLE_NOCOLOR='1')
    return subprocess.run(['ansible-playbook', '-i', str(inventory), str(play), *extra],
                          env=env, text=True, capture_output=True, timeout=300)


def metric(tmp, site):
    """The textfile the alert reads, written by the role's script for one site."""
    out = tmp / f'site-{site}.prom'
    result = subprocess.run(['bash', str(SCRIPT), 'prom', name(site), str(out)],
                            text=True, capture_output=True, timeout=120)
    assert result.returncode == 0, result.stdout + result.stderr
    assert PW not in out.read_text() and PW not in result.stdout + result.stderr, 'script printed a password'
    values = {}
    for line in out.read_text().splitlines():
        if line.startswith('spock_subscription_forward_origins{'):
            labels, value = line.rsplit(' ', 1)
            values[labels] = value
    return values


def expect_metric(tmp, value):
    for site in SITES:
        got = metric(tmp, site)
        want = {f'spock_subscription_forward_origins{{datname="{DB}",subscription_name='
                f'"sub_site_{provider}_to_site_{site}"}}': value
                for provider in SITES if provider != site}
        assert got == want, f'site {site}: got {got}, want {want}'


def main():
    if not IMAGE:
        sys.exit('set IMAGE to a postgres-spock image')
    try:
        up()
        with tempfile.TemporaryDirectory() as directory:
            tmp = Path(directory)

            wire("ARRAY['all']::text[]")
            result = run_role(tmp)
            output = result.stdout + result.stderr
            assert result.returncode != 0, 'the role accepted subscriptions with forward_origins set\n' + output
            assert 'GB-351' in output, output
            for provider, subscriber in subscriptions():
                assert f'sub_site_{provider}_to_site_{subscriber}' in output, \
                    f'refusal does not name sub_site_{provider}_to_site_{subscriber}\n' + output
            assert PW not in output, 'the role printed a password'
            print(next(line.strip() for line in output.splitlines() if 'GB-351' in line), flush=True)
            print('PASS forward_origins all on a three-site mesh: the install refuses, naming all six', flush=True)
            expect_metric(tmp, '1')
            print('PASS the metric reports 1 for every subscription at each site', flush=True)
            unwire()

            wire('ARRAY[]::text[]')
            result = run_role(tmp, '-e', '{"spock_mesh_forward_origins": ["all"]}')
            output = result.stdout + result.stderr
            assert result.returncode != 0, 'the role accepted spock_mesh_forward_origins: [all]\n' + output
            assert 'GB-351' in output and 'spock_mesh_forward_origins' in output, output
            print('PASS spock_mesh_forward_origins: [all] is refused', flush=True)
            for setting in ('[]', '["none"]'):
                result = run_role(tmp, '-e', f'{{"spock_mesh_forward_origins": {setting}, '
                                             f'"ory_spock_forward_origins": {setting}}}')
                assert result.returncode == 0, f'setting {setting} refused\n' + result.stdout + result.stderr
            print('PASS settings [] and [none] are accepted', flush=True)

            result = run_role(tmp)
            assert result.returncode == 0, 'the role refused a mesh with forward_origins empty\n' + \
                result.stdout + result.stderr
            print('PASS forward_origins empty on a three-site mesh: the install passes', flush=True)
            expect_metric(tmp, '0')
            print('PASS the metric reports 0 for every subscription at each site', flush=True)
    finally:
        for site in SITES:
            docker('rm', '-fv', name(site))
        docker('network', 'rm', NET)


if __name__ == '__main__':
    main()
