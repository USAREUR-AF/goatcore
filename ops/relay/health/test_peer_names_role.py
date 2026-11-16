#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""The relay-health peer-names deploy check (review of #977, items 6, 7, 8).

Evaluates the role's `when` conditions with jinja2 the way Ansible does:
- the probe proves the operator route the writer uses, not a weaker one;
- only a refused key (401/403) fails the deploy; a portal that is down or not
  found warns and lets the health dashboard deploy;
- the writer enabled with no key fails, unless owners are switched off.
"""
import os
import unittest

import jinja2
import yaml

HERE = os.path.dirname(os.path.abspath(__file__))
TASKS = os.path.join(HERE, "..", "..", "ansible", "roles", "relay-health", "tasks", "peer-names.yml")


def env():
    e = jinja2.Environment()
    e.filters["bool"] = lambda v: v if isinstance(v, bool) else str(v).strip().lower() in ("true", "yes", "1")
    return e


def holds(cond, **vars_):
    conds = cond if isinstance(cond, list) else [cond]
    e = env()
    return all(e.compile_expression(c.strip())(**vars_) for c in conds)


class PeerNamesRole(unittest.TestCase):
    def setUp(self):
        with open(TASKS) as f:
            self.block = yaml.safe_load(f)[0]["block"]
        self.by = {t["name"].split("— ", 1)[-1]: t for t in self.block}

    def task(self, start):
        hits = [t for k, t in self.by.items() if k.startswith(start)] or \
               [t for t in self.block if t["name"] == start]
        self.assertEqual(len(hits), 1, start)
        return hits[0]

    def test_the_probe_asks_the_operator_route_the_writer_uses(self):
        self.assertNotIn("assert-portal-key-accepted", open(TASKS).read(),
                         "GET /api/aliases admits any principal; the writer needs an operator key")
        u = self.task("ask the owner route")["ansible.builtin.uri"]
        self.assertEqual(u["method"], "POST")
        self.assertTrue(u["url"].endswith("/api/access/peers"))
        self.assertEqual(u["body"], {"netbird_names": []})

    def test_only_a_refused_key_fails_and_an_unreachable_portal_warns(self):
        fail = self.task("fail when the portal refuses")["when"]
        warn = self.task("say so when the portal cannot be reached")["when"]
        for status, fails, warns in ((401, True, False), (403, True, False), (200, False, False), (-1, False, True)):
            v = {"relay_peer_names_probe": {"status": status}, "relay_peer_names_portal_key": "k",
                 "relay_peer_names_probe_url": "http://p:8090"}
            self.assertEqual(holds(fail, **v), fails, status)
            self.assertEqual(holds(warn, **v), warns, status)
        nourl = {"relay_peer_names_portal_key": "k", "relay_peer_names_probe_url": ""}
        self.assertTrue(holds(warn, **nourl), "a portal container not found warns, it does not fail")

    def test_the_writer_enabled_without_a_key_fails_unless_owners_are_off(self):
        need = self.task("the writer needs a portal key")["when"]
        self.assertTrue(holds(need, relay_peer_names_portal_key=""))
        self.assertFalse(holds(need, relay_peer_names_portal_key="", relay_peer_names_owner_enabled=False))
        self.assertFalse(holds(need, relay_peer_names_portal_key="k"))

    # Review of the reconcile, item 10: the seed run after each deploy runs
    # with the same environment as the timer, owners and exceptions included.
    def test_the_seed_run_has_the_timers_environment(self):
        env_file = self.task("relay-health | peer-names resolver .env")
        keys = {line.split("=", 1)[0] for line in env_file["ansible.builtin.copy"]["content"].splitlines() if "=" in line}
        seed = self.task("relay-health | seed an initial peer-names resolve")
        self.assertEqual(set(seed["environment"]), keys)
        self.assertEqual(seed["environment"]["RELAY_PEER_NAMES_OWNER_EXCEPTIONS"],
                         [l for l in env_file["ansible.builtin.copy"]["content"].splitlines()
                          if l.startswith("RELAY_PEER_NAMES_OWNER_EXCEPTIONS=")][0].split("=", 1)[1])

    # Item 7: the file is checked at deploy, and no overlay file means none on the host.
    def test_the_exceptions_file_is_checked_and_follows_the_overlay(self):
        cp = self.task("the goatnet's owner exceptions")["ansible.builtin.copy"]
        self.assertIn("--check-exceptions %s", cp["validate"])
        rm = self.task("no owner exceptions in the overlay")
        self.assertEqual(rm["ansible.builtin.file"]["state"], "absent")
        self.assertTrue(holds(rm["when"], relay_peer_names_owner_exceptions_src=""))
        self.assertFalse(holds(rm["when"], relay_peer_names_owner_exceptions_src="files/x.json"))


if __name__ == "__main__":
    unittest.main()
