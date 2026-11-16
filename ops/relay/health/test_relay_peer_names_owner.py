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

    def test_an_older_portal_is_unread_not_none_or_roster(self):
        def old(base, key, names, **kw):
            return {n: peerowner._read({"resolved": True, "handle": "rivera-j",
                                        "person": {"ory_identity_id": "ory-1"}}, False) for n in names}

        c = cards()
        with patch.object(peerowner, "resolve", side_effect=old):
            read = rpn.enrich_owners(c)
        self.assertEqual(c["h2"]["owner"]["source"], "unread")
        self.assertIn("predates", c["h2"]["owner"]["reason"])
        self.assertEqual(read["state"], "unread", "a portal that cannot say is an unread cycle")

    def test_the_file_states_whether_owners_were_read(self):
        with tempfile.TemporaryDirectory() as d:
            rpn.LOCAL_DIR = d
            rpn.URLS = []
            rpn.publish({"h1": {"name": "x", "owner": {"source": "unread", "reason": "r"}}},
                        {"state": "unread", "reason": "r", "at": 1})
            with open(os.path.join(d, "peer-names.json")) as f:
                doc = json.load(f)
        self.assertEqual(doc["owner_read"]["state"], "unread")


class ReviewOfTheWriter(unittest.TestCase):
    """Each test here failed against the writer before its fix (review of #977)."""

    def setUp(self):
        rpn.peerowner = peerowner
        rpn.PORTAL_KEY = "k"
        rpn.PORTAL_URL = "http://portal.invalid"

    # 1: the portal's own owner sources failed -> unread, never "none".
    def test_failed_owner_sources_are_an_unread_cycle(self):
        body = {"peers": {"edge-gw-unlisted": {"resolved": False, "reason": "x",
                                               "owner": {"source": "none", "reason": "x"}}},
                "owner_sources": {"ok": False, "failed": ["registry_unavailable"]}}
        with patch.object(peerowner, "resolve",
                          side_effect=lambda b, k, names, **kw: peerowner.read_answer(body, names)):
            c = {"h3": {"id": "p3", "name": "edge-gw-unlisted"}}
            read = rpn.enrich_owners(c)
        self.assertEqual(c["h3"]["owner"]["source"], "unread")
        self.assertIn("registry_unavailable", c["h3"]["owner"]["reason"])
        self.assertEqual(read["state"], "unread")

    # 3: a 200 whose body is not the expected shape must not crash the writer.
    def test_a_malformed_answer_is_unread_not_a_crash(self):
        for body in ([], {"peers": []}, {"peers": {"a": "x"}}, "x"):
            rows = peerowner.read_answer(body, ["a"])
            self.assertEqual(rows["a"]["owner"]["source"], "unread", body)
        with patch.object(peerowner, "resolve", side_effect=ValueError("boom")):
            c = {"h": {"id": "p", "name": "a"}}
            read = rpn.enrich_owners(c)
        self.assertEqual((read["state"], c["h"]["owner"]["source"]), ("unread", "unread"))

    # 9: an owner block without a source is unread, not a bare "none".
    def test_an_owner_without_source_is_unread(self):
        rows = peerowner.read_answer({"peers": {"a": {"handle": "h", "owner": {"via": "x"}}}}, ["a"])
        self.assertEqual(rows["a"]["owner"]["source"], "unread")
        self.assertIn("malformed", rows["a"]["owner"]["reason"])

    # missing row for one name: unread for that card only; the cycle stays read.
    def test_a_missing_row_is_unread_for_that_peer_only(self):
        def some(b, k, names, **kw):
            return {"machine-anvil-pod-7": peerowner._read(
                {"resolved": True, "handle": "h", "machine": {"system_handle": "anvil-pod-7"},
                 "owner": {"source": "record"}}, False)}
        c = {"a": {"id": "p1", "name": "machine-anvil-pod-7"}, "b": {"id": "p2", "name": "edge"}}
        with patch.object(peerowner, "resolve", side_effect=some):
            read = rpn.enrich_owners(c)
        self.assertEqual(read["state"], "read")
        self.assertEqual(c["b"]["owner"]["source"], "unread")

    # 10: the names go to the portal in chunks under its body cap.
    def test_names_are_asked_in_chunks(self):
        calls = []

        class R:
            def __init__(self, b): self.b = b
            def __enter__(self): return self
            def __exit__(self, *a): return False
            def read(self, *a): return self.b

        def urlopen(req, timeout=0):
            names = json.loads(req.data)["netbird_names"]
            calls.append(len(names))
            return R(json.dumps({"peers": {n: {"owner": {"source": "none"}} for n in names},
                                 "owner_sources": {"ok": True, "failed": []}}).encode())
        with patch.object(peerowner.urllib.request, "urlopen", side_effect=urlopen):
            rows = peerowner.resolve("http://p", "k", [f"n{i}" for i in range(1201)], chunk=500)
        self.assertEqual(calls, [500, 500, 201])
        self.assertEqual(len(rows), 1201)

    # 2, 4, 5: the cloud body carries no person, identity, organization or
    # nationality; it carries the owner state, owner_read and generated_at; it
    # fits the ingest cap at a realistic size; an over-cap body is a failure.
    def card(self, i):
        return {"name": f"machine-box-{i}", "id": f"p{i}", "aff": "ZAGROS-OBSERVER", "ip": "100.127.0.9",
                "os": "linux", "host": f"box-{i}", "seen": "2026-01-05", "conn": True, "ory_id": "ory-x",
                "owner": {"source": "record", "via": "machine-custody",
                          "person": {"handle": "rivera-j", "identity_id": "ory-x"},
                          "org": {"id": "org-anvil", "nation": "XAA"},
                          "nationality": {"value": "XCC", "state": "recorded"}}}

    def test_the_cloud_body_carries_owner_state_only(self):
        body = json.loads(rpn.remote_body({"h": self.card(1)}, {"state": "read", "at": 1}, 7))
        txt = json.dumps(body)
        for leaked in ("rivera-j", "ory-x", "org-anvil", "XAA", "XCC", "identity_id", "nationality"):
            self.assertNotIn(leaked, txt, leaked)
        self.assertEqual(body["h"]["owner"], {"source": "record", "exception": False})
        self.assertEqual(body[rpn.REMOTE_META_KEY]["generated_at"], 7)
        self.assertEqual(body[rpn.REMOTE_META_KEY]["owner_read"]["state"], "read")

    def test_the_cloud_body_fits_the_cap_at_size_and_over_cap_is_a_failure(self):
        names = {f"h{i}": self.card(i) for i in range(500)}
        self.assertLess(len(rpn.remote_body(names, {"state": "read"}, 1)), rpn.REMOTE_MAX_BYTES)
        rpn.URLS, rpn.LOCAL_DIR = ["http://cloud.invalid/relay-metrics-ingest"], ""
        old = rpn.REMOTE_MAX_BYTES
        try:
            rpn.REMOTE_MAX_BYTES = 100
            with patch.object(rpn.urllib.request, "urlopen") as u:
                self.assertEqual(rpn.publish(names, {"state": "read"}), 1)
                u.assert_not_called()
        finally:
            rpn.REMOTE_MAX_BYTES, rpn.URLS = old, []


class Reconcile(unittest.TestCase):
    """ADR 1249 decisions 5 and 7: the metric whose expected value is 0, and
    owner exceptions that are shown and counted, never hidden."""

    def cards(self):
        return {
            "h1": {"id": "p1", "name": "machine-anvil-pod-7", "conn": True, "owner": {"source": "record"}},
            "h2": {"id": "p2", "name": "peer-rivera-j-zagros-2026-02-09-laptop", "conn": True, "owner": {"source": "roster"}},
            "h3": {"id": "p3", "name": "edge-gw-unlisted", "conn": True, "owner": {"source": "none"}},
            "h4": {"id": "p4", "name": "old-kiosk", "conn": False, "owner": {"source": "none"}},
            "h5": {"id": "p5", "name": "infra-dns-a", "conn": True, "owner": {"source": "declared"}},
            "h6": {"id": "p6", "name": "machine-was-excepted", "conn": True, "owner": {"source": "record"}},
        }

    def exc(self):
        e = lambda pid, name: {"peer_id": pid, "name": name, "reason": "present before the rule",
                               "granted_by": "ops-lead", "granted_on": "2026-01-05"}
        return {"p4": e("p4", "old-kiosk"), "p6": e("p6", "machine-was-excepted"),
                "p9": e("p9", "left-the-mesh")}

    def test_only_the_record_or_a_declaration_is_compliant_and_an_exception_is_its_own_state(self):
        g = {k: rpn.grade(c, self.exc()) for k, c in self.cards().items()}
        self.assertEqual(g, {"h1": "recorded", "h2": "lead", "h3": "unrecorded",
                             "h4": "excepted", "h5": "recorded", "h6": "exception_stale"})

    def test_an_exception_matches_the_peer_id_not_the_name(self):
        renamed = {"x": {"id": "p4", "name": "renamed-kiosk", "owner": {"source": "none"}}}
        self.assertEqual(rpn.grade(renamed["x"], self.exc()), "excepted")
        same_name = {"x": {"id": "other", "name": "old-kiosk", "owner": {"source": "none"}}}
        self.assertEqual(rpn.grade(same_name["x"], self.exc()), "unrecorded")

    def test_the_card_shows_the_exception_with_its_reason(self):
        cards = self.cards()
        rpn.mark_exceptions(cards, self.exc())
        self.assertEqual(cards["h4"]["owner_exception"]["reason"], "present before the rule")
        self.assertEqual(cards["h4"]["owner_exception"]["state"], "active")
        self.assertEqual(cards["h6"]["owner_exception"]["state"], "stale")
        self.assertNotIn("owner_exception", cards["h3"])
        self.assertNotIn("review_by", cards["h4"]["owner_exception"], "no date is invented")

    def test_the_metric_counts_exceptions_apart_and_reports_stale_ones(self):
        text = rpn.owner_metrics(self.cards(), {"state": "read"}, self.exc(), now=5)
        self.assertIn('goat_peer_owner_unrecorded{class="person"} 1', text)
        self.assertIn('goat_peer_owner_unrecorded{class="other"} 1', text)
        self.assertIn('goat_peer_owner_excepted{class="other"} 1', text)
        self.assertIn('goat_peer_owner_exception_stale{reason="recorded"} 1', text)
        self.assertIn('goat_peer_owner_exception_stale{reason="peer_gone"} 1', text)
        self.assertIn("goat_peer_owner_read 1", text)
        self.assertNotIn("edge-gw-unlisted", text, "names stay out of the metric")

    def test_an_unread_cycle_writes_no_fresh_count(self):
        # Review item 3: an unread cycle used to write unrecorded 0, which
        # cleared the critical page. It carries the last read count, or none.
        cards = {k: dict(c, owner={"source": "unread", "reason": "401"}) for k, c in self.cards().items()}
        text = rpn.owner_metrics(cards, {"state": "unread"}, {}, now=5)
        self.assertIn("goat_peer_owner_read 0", text)
        self.assertNotIn("goat_peer_owner_unrecorded{", text)
        before = rpn.owner_metrics(self.cards(), {"state": "read"}, {}, now=3)
        text = rpn.owner_metrics(cards, {"state": "unread"}, {}, now=5, previous=before)
        self.assertIn('goat_peer_owner_unrecorded{class="other"} 2', text)
        self.assertIn("goat_peer_owner_counts_timestamp_seconds 3", text)
        self.assertIn("goat_peer_owner_last_run_timestamp_seconds 5", text)

    def test_an_incomplete_exception_is_refused_and_the_file_must_carry_its_schema(self):
        with tempfile.TemporaryDirectory() as d:
            path = os.path.join(d, "exc.json")
            with open(path, "w") as f:
                json.dump({"schema": rpn.EXCEPTIONS_SCHEMA, "exceptions": [
                    {"peer_id": "p4", "name": "old-kiosk", "reason": "r", "granted_by": "ops-lead", "granted_on": "2026-01-05"},
                    {"peer_id": "p3", "name": "edge-gw-unlisted", "reason": "r"}]}, f)
            self.assertEqual(sorted(rpn.load_exceptions(path)), ["p4"])
            with open(path, "w") as f:
                json.dump({"exceptions": []}, f)
            self.assertEqual(rpn.load_exceptions(path), {})

    def test_the_initial_file_is_written_once_never_blind_and_without_a_review_date(self):
        with tempfile.TemporaryDirectory() as d:
            path = os.path.join(d, "exc.json")
            with self.assertRaises(SystemExit):
                rpn.write_exceptions(self.cards(), {"state": "unread"}, path, "ops-lead", "r", "2026-01-05")
            with self.assertRaises(SystemExit):
                rpn.write_exceptions(self.cards(), {"state": "read"}, path, "", "r", "2026-01-05")
            # The operator states the date; the tool never picks one.
            for bad in (None, "", "2026-1-5", "2026-02-30"):
                with self.assertRaises(SystemExit):
                    rpn.write_exceptions(self.cards(), {"state": "read"}, path, "ops-lead", "r", bad)
                self.assertFalse(os.path.exists(path))
            rpn.write_exceptions(self.cards(), {"state": "read"}, path, "ops-lead",
                                 "present before the owner rule", "2026-01-05")
            with open(path) as f:
                doc = json.load(f)
            self.assertEqual(doc["schema"], rpn.EXCEPTIONS_SCHEMA)
            self.assertEqual([e["peer_id"] for e in doc["exceptions"]], ["p3", "p4", "p2"])
            self.assertTrue(all("review_by" not in e for e in doc["exceptions"]))
            self.assertTrue(all(e["granted_on"] == "2026-01-05" for e in doc["exceptions"]))
            with self.assertRaises(SystemExit):
                rpn.write_exceptions(self.cards(), {"state": "read"}, path, "ops-lead", "r", "2026-01-05")
            self.assertEqual(sorted(rpn.load_exceptions(path)), ["p2", "p3", "p4"])

    def test_the_public_example_validates_against_the_schema_shape(self):
        here = os.path.join(HERE, "..", "..", "identity")
        with open(os.path.join(here, "peer-owner-exceptions.example.json")) as f:
            ex = json.load(f)
        with open(os.path.join(here, "peer-owner-exceptions.schema.json")) as f:
            sch = json.load(f)
        self.assertEqual(ex["schema"], sch["properties"]["schema"]["const"])
        req = sch["properties"]["exceptions"]["items"]["required"]
        allowed = set(sch["properties"]["exceptions"]["items"]["properties"])
        for e in ex["exceptions"]:
            self.assertTrue(all(k in e for k in req))
            self.assertTrue(set(e) <= allowed)
        with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as f:
            json.dump(ex, f)
        try:
            self.assertEqual(len(rpn.load_exceptions(f.name)), 2)
        finally:
            os.unlink(f.name)


if __name__ == "__main__":
    unittest.main()
