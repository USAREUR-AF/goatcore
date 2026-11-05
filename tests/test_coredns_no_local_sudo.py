#!/usr/bin/env python3
"""goat-coredns.yml runs with become: true for the mgmt hosts, and its NetBird API calls are
delegated to the operator's machine. Each delegated task must set become: false: inherited, it
runs sudo on the operator's machine for an HTTPS call, and a laptop without passwordless sudo
fails the play ("sudo: a password is required").

Run: python3 -m unittest tests/test_coredns_no_local_sudo.py -v
"""
import unittest
from pathlib import Path

import yaml

PLAY = Path(__file__).resolve().parents[1] / "ops/ansible/goat-coredns.yml"


def tasks(items):
    for t in items or []:
        if "block" in t:
            yield from tasks(t["block"])
            yield from tasks(t.get("rescue"))
            yield from tasks(t.get("always"))
        else:
            yield t


class NoLocalSudo(unittest.TestCase):
    def test_localhost_tasks_do_not_become(self):
        bad, seen = [], 0
        for play in yaml.safe_load(PLAY.read_text()):
            for key in ("pre_tasks", "tasks", "post_tasks"):
                for t in tasks(play.get(key)):
                    if t.get("delegate_to") in ("localhost", "127.0.0.1"):
                        seen += 1
                        if t.get("become", play.get("become")) is not False:
                            bad.append(t.get("name"))
        self.assertGreater(seen, 0, "no localhost-delegated task found; the test is miswired")
        self.assertEqual(bad, [], "localhost tasks that would sudo on the operator's machine")


if __name__ == "__main__":
    unittest.main()
