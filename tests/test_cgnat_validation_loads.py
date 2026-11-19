"""Playbooks that import mgmt-host.yml load with no extra variables, and
cgnat-validation.yml's header names only files that exist.

mgmt-host.yml's play targets `mgmt_host_target | mandatory`, so each playbook
that imports it must supply that target itself. The test lists each such
playbook's hosts against an inventory with the groups the playbook uses, with
no extra variables, and checks that the mgmt-plane play covers the playbook's
mgmt hosts. It runs nothing on a host, and nothing from ops/ansible.
"""
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
ANSIBLE = ROOT / 'ops/ansible'
PLAYBOOK = ANSIBLE / 'cgnat-validation.yml'

# (playbook, its inventory groups -> hosts, the hosts its mgmt-plane play covers)
IMPORTERS = [
    ('cgnat-validation.yml',
     {'cgnat_mgmt_a_hosts': ['cgnat-mgmt-a'], 'cgnat_mgmt_b_hosts': ['cgnat-mgmt-b']},
     ['cgnat-mgmt-a', 'cgnat-mgmt-b']),
    ('goat-relay-tier4b.yml', {'goat_relay_tier4b_mgmt': ['tier4b-mgmt']}, ['tier4b-mgmt']),
    ('pgbackrest-wave18.yml', {'pgbackrest_wave18_hosts': ['wave18-mgmt']}, ['wave18-mgmt']),
]


def inventory(groups):
    return ''.join(f'[{g}]\n' + ''.join(f'{h} ansible_connection=local\n' for h in hosts) + '\n'
                   for g, hosts in groups.items())


def require_ansible_playbook(case):
    if shutil.which('ansible-playbook'):
        return
    if os.environ.get('GOAT_TEST_REQUIRE_ANSIBLE') == '1':
        case.fail('GOAT_TEST_REQUIRE_ANSIBLE=1 but ansible-playbook is not on PATH')
    case.skipTest('ansible-playbook is required')


def header(path):
    lines = []
    for line in path.read_text().splitlines()[1:]:
        if not line.startswith('#'):
            break
        lines.append(line)
    return '\n'.join(lines)


class CgnatValidationLoads(unittest.TestCase):

    def test_lists_hosts_with_no_extra_vars(self):
        require_ansible_playbook(self)
        for name, groups, mgmt in IMPORTERS:
            with self.subTest(name), tempfile.TemporaryDirectory() as d:
                inv = Path(d) / 'inventory.ini'
                inv.write_text(inventory(groups))
                env = dict(os.environ, ANSIBLE_BECOME='False', ANSIBLE_NOCOLOR='1',
                           ANSIBLE_LOCALHOST_WARNING='False')
                env.pop('ANSIBLE_CONFIG', None)
                r = subprocess.run(['ansible-playbook', '--list-hosts', '-i', str(inv),
                                    str(ANSIBLE / name)],
                                   cwd=d, env=env, capture_output=True, text=True, timeout=300)
                out = r.stdout + r.stderr
                self.assertEqual(r.returncode, 0, f'{name} does not load:\n{out[-4000:]}')
                play = re.search(r'play #\d+ \([^)]*\): Deploy management-plane compose stack'
                                 r'.*?(?=\n  play #|\Z)', r.stdout, re.S)
                self.assertIsNotNone(play, f'{name}: no mgmt-plane play listed:\n{r.stdout}')
                for host in mgmt:
                    self.assertIn(host, play.group(0))

    def test_header_names_only_files_that_exist(self):
        text = header(PLAYBOOK)
        paths = set(re.findall(r'[\w./-]+\.(?:md|yml|yaml|sh|tf|ini)\b', text))
        self.assertTrue(paths, 'the header names no files')
        missing = sorted(p for p in paths
                         if not any((base / p).exists() for base in (ROOT, ANSIBLE)))
        self.assertEqual(missing, [], f'the header names files that do not exist: {missing}')


if __name__ == '__main__':
    unittest.main()
