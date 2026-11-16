#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""What relay-peer-names writes about each peer's owner.

Pinned: every card leaves with an owner state; a portal that cannot be asked
is `unread` with its reason on every card and once in the file, never an
absent owner; every peer is asked about, machines and infrastructure included.
Synthetic names only.
"""
import importlib.util
import json
import os
import sys
import tempfile
import unittest
from unittest.mock import patch

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "..", "identity"))
import peerowner  # noqa: E402

spec = importlib.util.spec_from_file_location("relay_peer_names", os.path.join(HERE, "relay-peer-names.py"))
rpn = importlib.util.module_from_spec(spec)
spec.loader.exec_module(rpn)


def cards():
    return {
        "h1": {"name": "machine-anvil-pod-7-rack2"},
        "h2": {"name": "peer-rivera-j-zagros-2026-02-09-laptop"},
        "h3": {"name": "edge-gw-unlisted"},
    }


RECORD = {"source": "record", "via": "machine-custody",
          "person": {"handle": "rivera-j", "identity_id": "ory-1", "disabled": False, "email": "x"},
          "org": {"id": "org-anvil", "nation": "XAA", "state": "recorded"},
          "nationality": {"value": "", "state": "not_stamped", "reason": "not yet stamped"},
          "machine_unit": "1st Anvil Bn"}


class Owners(unittest.TestCase):
    def setUp(self):
        rpn.peerowner = peerowner
        rpn.PORTAL_KEY = "k"
        rpn.PORTAL_URL = "http://portal.invalid"

    def test_every_peer_is_asked_about_and_carries_its_owner(self):
        asked = {}

        def resolve(base, key, names, **kw):
            asked["names"] = names
            wire = {
                "machine-anvil-pod-7-rack2": {"resolved": True, "handle": "rivera-j",
                                              "machine": {"system_handle": "anvil-pod-7"},
                                              "person": {"ory_identity_id": "ory-1"}, "owner": RECORD},
                "peer-rivera-j-zagros-2026-02-09-laptop": {"resolved": True, "handle": "rivera-j",
                                                           "person": {"ory_identity_id": "ory-1"},
                                                           "owner": dict(RECORD, via="slot-identity")},
                "edge-gw-unlisted": {"resolved": False, "reason": "no slot handle matches",
                                     "owner": {"source": "none", "reason": "no slot handle matches"}},
            }
            return {n: peerowner._read(wire[n], False) for n in names}

        c = cards()
        with patch.object(peerowner, "resolve", side_effect=resolve):
            read = rpn.enrich_owners(c)
        self.assertEqual(sorted(asked["names"]), sorted(x["name"] for x in cards().values()),
                         "machines and infrastructure are asked about, not only peer- names")
        self.assertEqual(read["state"], "read")
        self.assertEqual(read["counts"], {"record": 2, "none": 1})
        self.assertEqual(c["h1"]["owner"]["org"]["nation"], "XAA")
        self.assertEqual(c["h1"]["owner"]["machine_unit"], "1st Anvil Bn")
        self.assertEqual(c["h1"]["ory_id"], "ory-1")
        self.assertNotIn("email", c["h1"]["owner"]["person"], "the owner block carries no e-mail")
        self.assertEqual(c["h3"]["owner"]["source"], "none")

    def test_a_refused_key_is_unread_on_every_card_and_in_the_file(self):
        def refused(base, key, names, **kw):
            return {n: peerowner._row(peerowner.UNREAD, reason="portal answered HTTP 401") for n in names}

        c = cards()
        with patch.object(peerowner, "resolve", side_effect=refused):
            read = rpn.enrich_owners(c)
        self.assertEqual(read["state"], "unread")
        self.assertIn("401", read["reason"])
        for card in c.values():
            self.assertEqual(card["owner"], {"source": "unread", "reason": "portal answered HTTP 401"})
            self.assertNotIn("ory_id", card)

    def test_no_key_on_the_host_is_unread_not_silent(self):
        rpn.PORTAL_KEY = ""
        c = cards()
        read = rpn.enrich_owners(c)
        self.assertEqual(read["state"], "unread")
        self.assertTrue(all(x["owner"]["source"] == "unread" for x in c.values()))

    def test_an_older_portal_is_labeled_roster_not_record(self):
        def old(base, key, names, **kw):
            return {n: peerowner._read({"resolved": True, "handle": "rivera-j",
                                        "person": {"ory_identity_id": "ory-1"}}, False) for n in names}

        c = cards()
        with patch.object(peerowner, "resolve", side_effect=old):
            rpn.enrich_owners(c)
        self.assertEqual(c["h2"]["owner"]["source"], "roster")

    def test_the_file_states_whether_owners_were_read(self):
        with tempfile.TemporaryDirectory() as d:
            rpn.LOCAL_DIR = d
            rpn.URLS = []
            rpn.publish({"h1": {"name": "x", "owner": {"source": "unread", "reason": "r"}}},
                        {"state": "unread", "reason": "r", "at": 1})
            with open(os.path.join(d, "peer-names.json")) as f:
                doc = json.load(f)
        self.assertEqual(doc["owner_read"]["state"], "unread")


if __name__ == "__main__":
    unittest.main()
