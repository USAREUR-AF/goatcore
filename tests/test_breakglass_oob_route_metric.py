"""A break-glass node's out-of-band interface takes a strictly higher route
metric than its primary (ADR 1114 §5): a reach-in path, never the default
route.

Each case runs the breakglass-node role's out-of-band route tasks on localhost
with stand-in `nmcli`, `networkctl` and `ip` on PATH and the systemd-networkd
drop-in directory redirected into the test's own directory. Nothing touches
the host's network, and nothing runs from ops/ansible.
"""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

import yaml

ROOT = Path(__file__).resolve().parents[1]
ROLE = ROOT / 'ops/ansible/roles/breakglass-node'
OOB = 'wwan0'
PRIMARY = 'eth0'

# Records every call. Which stack owns the interface comes from $FAKE_STACK;
# the out-of-band default route's metric is $FAKE_OOB_BEFORE until a reconfigure
# or reapply has run, then $FAKE_OOB_AFTER. The primary's is $FAKE_PRIMARY.
STANDIN = r'''#!/usr/bin/env bash
tool=$(basename "$0")
printf '%s %s\n' "$tool" "$*" >> "$FAKE_LOG"
case "$tool" in
  nmcli)
    [ "$FAKE_STACK" = nm ] || exit 10
    case "$*" in
      *"device show"*) echo "oob-uplink" ;;
      *"connection show"*) echo "-1"; echo "-1" ;;
      *"device reapply"*) touch "$FAKE_DIR/applied" ;;
    esac ;;
  networkctl)
    case "$*" in
      status*)
        [ "$FAKE_STACK" = networkd ] || { echo "Network File: n/a"; exit 0; }
        echo "       State: routable (configured)"
        echo "Network File: /run/systemd/network/10-netplan-$FAKE_OOB.network" ;;
      reconfigure*) touch "$FAKE_DIR/applied" ;;
    esac ;;
  ip)
    oob=$FAKE_OOB_BEFORE
    [ -e "$FAKE_DIR/applied" ] && oob=$FAKE_OOB_AFTER
    case "$*" in
      *-4*) printf '[{"dst":"default","gateway":"192.0.2.1","dev":"%s","metric":%s},{"dst":"default","gateway":"198.51.100.1","dev":"%s","metric":%s}]\n' \
              "$FAKE_PRIMARY_DEV" "$FAKE_PRIMARY" "$FAKE_OOB" "$oob" ;;
      *) echo '[]' ;;
    esac ;;
esac
exit 0
'''


def require_ansible_playbook(case):
    if shutil.which('ansible-playbook'):
        return
    if os.environ.get('GOAT_TEST_REQUIRE_ANSIBLE') == '1':
        case.fail('GOAT_TEST_REQUIRE_ANSIBLE=1 but ansible-playbook is not on PATH')
    case.skipTest('ansible-playbook is required')


class OutOfBandRouteMetric(unittest.TestCase):

    def run_tasks(self, stack, oob_before=100, oob_after=1000, primary=100, metric=None):
        require_ansible_playbook(self)
        with tempfile.TemporaryDirectory() as d:
            d = Path(d)
            bindir, dropins = d / 'bin', d / 'network'
            bindir.mkdir()
            dropins.mkdir()
            for tool in ('nmcli', 'networkctl', 'ip'):
                (bindir / tool).write_text(STANDIN)
                (bindir / tool).chmod(0o755)
            extra = {'breakglass_oob_interface': OOB,
                     'breakglass_oob_networkd_dropin_root': str(dropins),
                     # Two reads one second apart: the stand-in's routes are final.
                     'breakglass_oob_route_check_retries': 1,
                     'breakglass_oob_route_check_delay': 1}
            if metric is not None:
                extra['breakglass_oob_route_metric'] = metric
            play = [{'hosts': 'localhost', 'connection': 'local', 'gather_facts': False,
                     'vars': extra,
                     'tasks': [{'ansible.builtin.include_role': {
                         'name': str(ROLE), 'tasks_from': 'oob-route-metric'}}]}]
            (d / 'play.yml').write_text(yaml.safe_dump(play))
            env = dict(os.environ, PATH=f'{bindir}:{os.environ["PATH"]}',
                       FAKE_LOG=str(d / 'log'), FAKE_DIR=str(d), FAKE_STACK=stack,
                       FAKE_OOB=OOB, FAKE_PRIMARY_DEV=PRIMARY,
                       FAKE_OOB_BEFORE=str(oob_before), FAKE_OOB_AFTER=str(oob_after),
                       FAKE_PRIMARY=str(primary), ANSIBLE_BECOME='False',
                       ANSIBLE_NOCOLOR='1', ANSIBLE_RETRY_FILES_ENABLED='False',
                       ANSIBLE_LOCALHOST_WARNING='False',
                       ANSIBLE_INVENTORY_UNPARSED_WARNING='False')
            env.pop('ANSIBLE_CONFIG', None)
            r = subprocess.run(['ansible-playbook', '-i', 'localhost,', 'play.yml'], cwd=d,
                               env=env, capture_output=True, text=True, timeout=300)
            log = (d / 'log').read_text() if (d / 'log').exists() else ''
            files = {str(p.relative_to(dropins)): p.read_text() for p in dropins.rglob('*') if p.is_file()}
            return r, log, files

    def test_networkd_gets_a_drop_in_with_the_metric(self):
        r, log, files = self.run_tasks('networkd')
        self.assertEqual(r.returncode, 0, r.stdout[-4000:])
        conf = files.get('10-netplan-wwan0.network.d/90-breakglass-oob-route-metric.conf', '')
        self.assertIn('[DHCPv4]\nRouteMetric=1000', conf)
        self.assertIn('[IPv6AcceptRA]\nRouteMetric=1000', conf)
        self.assertIn('networkctl reload', log)
        self.assertIn(f'networkctl reconfigure {OOB}', log)
        self.assertNotIn('nmcli connection modify', log)

    def test_networkmanager_gets_the_metric_on_the_connection(self):
        r, log, files = self.run_tasks('nm')
        self.assertEqual(r.returncode, 0, r.stdout[-4000:])
        self.assertIn('nmcli connection modify oob-uplink ipv4.route-metric 1000 ipv6.route-metric 1000', log)
        self.assertIn(f'nmcli device reapply {OOB}', log)
        self.assertEqual(files, {})

    def test_the_metric_is_a_default_the_deployment_overrides(self):
        r, log, files = self.run_tasks('networkd', oob_after=4000, metric=4000)
        self.assertEqual(r.returncode, 0, r.stdout[-4000:])
        self.assertIn('RouteMetric=4000', files['10-netplan-wwan0.network.d/90-breakglass-oob-route-metric.conf'])
        defaults = yaml.safe_load((ROLE / 'defaults/main.yml').read_text())
        self.assertEqual(defaults['breakglass_oob_interface'], '')
        self.assertIsInstance(defaults['breakglass_oob_route_metric'], int)

    def test_no_stack_owns_the_interface_refuses(self):
        r, log, files = self.run_tasks('none')
        self.assertNotEqual(r.returncode, 0)
        self.assertIn(OOB, r.stdout)
        self.assertIn('neither NetworkManager nor systemd-networkd', r.stdout)
        self.assertEqual(files, {})

    def test_a_primary_at_or_above_the_metric_fails_the_converge(self):
        r, log, files = self.run_tasks('networkd', primary=1000)
        self.assertNotEqual(r.returncode, 0)
        self.assertIn('strictly higher', r.stdout)
        self.assertIn(PRIMARY, r.stdout)

    def test_the_role_runs_the_metric_tasks_only_when_an_interface_is_named(self):
        tasks = yaml.safe_load((ROLE / 'tasks/main.yml').read_text())
        hits = [t for t in tasks if isinstance(t, dict)
                and 'oob-route-metric' in json.dumps(t.get('ansible.builtin.include_tasks', ''))]
        self.assertEqual(len(hits), 1, 'tasks/main.yml does not include oob-route-metric.yml once')
        self.assertIn('breakglass_oob_interface', json.dumps(hits[0].get('when', '')))


if __name__ == '__main__':
    unittest.main()
