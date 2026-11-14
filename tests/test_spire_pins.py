"""The published stage files must form a complete path to the fleet pair."""
from pathlib import Path
import re
import json
import os
import shutil
import subprocess
import unittest

import yaml

REPO = Path(__file__).resolve().parents[1]


class UpgradePins(unittest.TestCase):
    def test_stages_are_consecutive_and_end_at_the_declared_pair(self):
        stages = sorted((REPO / 'ops/pins/spire-upgrades').glob('*.yml'),
                        key=lambda p: tuple(map(int, p.stem.split('.'))))
        versions = [tuple(map(int, p.stem.split('.'))) for p in stages]
        self.assertEqual(versions[0], (1, 11, 2))
        self.assertEqual(versions[-1], (1, 16, 0))
        self.assertTrue(all(b[0] == a[0] and b[1] - a[1] == 1
                            for a, b in zip(versions, versions[1:])))
        for path in stages:
            pair = yaml.safe_load(path.read_text())
            self.assertEqual(set(pair), {'spire_server_image', 'spire_agent_image'})
            for kind in ('server', 'agent'):
                self.assertRegex(pair['spire_' + kind + '_image'],
                                 '^ghcr.io/spiffe/spire-' + kind + ':' + re.escape(path.stem)
                                 + '@sha256:[a-f0-9]{64}$')
        final = yaml.safe_load(stages[-1].read_text())
        components = yaml.safe_load((REPO / 'ops/pins/fleet-pins.yaml').read_text())['components']
        self.assertEqual(components['spire']['fleet'], final['spire_server_image'])
        self.assertEqual(components['spire-agent']['fleet'], final['spire_agent_image'])
        for name in ('docker-compose.spire.yml', 'docker-compose.spire-agent.yml'):
            text = (REPO / 'ops/compose/mgmt-host' / name).read_text()
            self.assertIn(final['spire_agent_image'], text)
            if name == 'docker-compose.spire.yml':
                self.assertIn(final['spire_server_image'], text)

    def test_spire_cannot_use_automatic_image_only_rollback(self):
        meta = yaml.safe_load((REPO / 'ops/ansible/roles/spire/meta/main.yml').read_text())
        rollback = next(d['vars'] for d in meta['dependencies'] if d['role'] == 'rollback-ledger')
        self.assertEqual(rollback['rollback_artifact_type'], 'image')
        self.assertFalse(rollback['rollback_reversible'])
        self.assertIn('spire-upgrades.md', rollback['rollback_restore_pointer'])

    def test_declared_restore_only_policy_refuses_before_host_ledger_reads(self):
        self.assertIsNotNone(shutil.which('jq'), 'jq is required for rollback safety checks')
        source = (REPO / 'ops/scripts/rollback.sh').read_text()
        start = source.index('refuse_declared_irreversible() {')
        end = source.index('\n}\n', start) + 3
        guard = source[start:end]
        meta = yaml.safe_load((REPO / 'ops/ansible/roles/spire/meta/main.yml').read_text())
        rows = [(json.dumps(meta), 3),
                (json.dumps({'dependencies': [{'role': 'rollback-ledger', 'vars': {'rollback_reversible': True}}]}), 0),
                (json.dumps({'dependencies': []}), 0), ('{broken', 1), ('[]', 1),
                (json.dumps(meta) + ' ' + json.dumps(meta), 1)]
        for raw, expected in rows:
            script = ('set -eu\nROLE_DIR="' + str(REPO / 'ops/ansible/roles/spire') + '"; SERVICE=spire; EX_REFUSE_UNSAFE=3\n'
                      'ledger_json() { printf "%s" "$ROW"; }\n'
                      'die() { printf "%s\\n" "$*" >&2; exit 1; }\n'
                      'note() { printf "%s\\n" "$*" >&2; }\n'
                      'host_latest_row() { echo unexpected-host-read; exit 99; }\n'
                      + guard + '\nrefuse_declared_irreversible\n')
            result = subprocess.run(['bash', '-c', script], env=dict(os.environ, ROW=raw),
                                    capture_output=True, text=True, timeout=10)
            self.assertEqual(result.returncode, expected, result.stdout + result.stderr)
            self.assertNotIn('unexpected-host-read', result.stdout + result.stderr)
            if expected == 3:
                self.assertIn('spire-upgrades.md', result.stderr)



if __name__ == '__main__':
    unittest.main()
