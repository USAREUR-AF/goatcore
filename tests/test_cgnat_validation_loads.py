"""cgnat-validation.yml loads as its header says to run it, and its header
names only files that exist.

The playbook imports mgmt-host.yml, whose play targets
`mgmt_host_target | mandatory`; cgnat-validation.yml must supply that target
itself, so the run line in its header (with no -e for it) works. The test lists
the playbook's hosts against an inventory with the two groups the playbook
uses, with no extra variables, and checks that the mgmt-plane play covers both
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

INVENTORY = """[cgnat_mgmt_a_hosts]
cgnat-mgmt-a ansible_connection=local

[cgnat_mgmt_b_hosts]
cgnat-mgmt-b ansible_connection=local
"""


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
        with tempfile.TemporaryDirectory() as d:
            inv = Path(d) / 'inventory.ini'
            inv.write_text(INVENTORY)
            env = dict(os.environ, ANSIBLE_BECOME='False', ANSIBLE_NOCOLOR='1',
                       ANSIBLE_LOCALHOST_WARNING='False')
            env.pop('ANSIBLE_CONFIG', None)
            r = subprocess.run(['ansible-playbook', '--list-hosts', '-i', str(inv), str(PLAYBOOK)],
                               cwd=d, env=env, capture_output=True, text=True, timeout=300)
        out = r.stdout + r.stderr
        self.assertEqual(r.returncode, 0, f'cgnat-validation.yml does not load:\n{out[-4000:]}')
        play = re.search(r'play #1 \([^)]*\): Deploy management-plane compose stack.*?(?=\n  play #|\Z)',
                         r.stdout, re.S)
        self.assertIsNotNone(play, f'no mgmt-plane play listed:\n{r.stdout}')
        for host in ('cgnat-mgmt-a', 'cgnat-mgmt-b'):
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
