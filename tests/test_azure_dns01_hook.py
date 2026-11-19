"""Offline failure and concurrency controls for Azure DNS publication."""
import importlib.util
from pathlib import Path
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('dns_hook', ROOT / 'ops/ansible/templates/certbot-azure-dns01-hook.py')
hook = importlib.util.module_from_spec(spec)
spec.loader.exec_module(hook)

class Updates(unittest.TestCase):
    cfg = {'subscription_id': 'fixture', 'resource_group': 'fixture', 'zone_name': 'example.org'}

    def test_put_not_found_is_failure(self):
        with patch.object(hook, '_arm', side_effect=[(404, {}), (404, {})]):
            with self.assertRaises(hook.HookError):
                hook._update(self.cfg, 'fixture', '_acme-challenge', lambda r: hook.add_value(r, 'token'))

    def test_conflict_reloads_and_preserves_other_challenge(self):
        current = {'etag': 'etag-2', 'properties': {'TXTRecords': [{'value': ['other']}]}}
        with patch.object(hook, '_arm', side_effect=[(404, {}), (412, {}), (200, current), (200, {})]) as arm:
            hook._update(self.cfg, 'fixture', '_acme-challenge', lambda r: hook.add_value(r, 'token'))
        write = arm.call_args_list[-1].args
        self.assertEqual(write[3]['properties']['TXTRecords'], [{'value': ['other']}, {'value': ['token']}])
        self.assertEqual(write[4], {'If-Match': 'etag-2'})

    def test_cleanup_missing_record_does_not_write(self):
        with patch.object(hook, '_arm', return_value=(404, {})) as arm:
            hook._update(self.cfg, 'fixture', '_acme-challenge', lambda r: hook.remove_value(r, 'token'))
        self.assertEqual(arm.call_count, 1)

    def test_outside_zone_refused(self):
        with self.assertRaises(hook.HookError):
            hook.relative_name('other.example.net', 'example.org')

if __name__ == '__main__':
    unittest.main()
