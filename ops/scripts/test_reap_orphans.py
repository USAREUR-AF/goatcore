#!/usr/bin/env python3
"""Tests for reap-orphans.py — fixture DBs + pure classify() coverage.

Run: python3 ops/scripts/test_reap_orphans.py
"""
from __future__ import annotations

import importlib.util
import json
import sqlite3
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("reap_orphans", HERE / "reap-orphans.py")
reap = importlib.util.module_from_spec(spec)
spec.loader.exec_module(reap)


def make_onboard_db(path: Path, rows: list[tuple]) -> None:
    """rows: (id, handle, email, status, env, created_at[, manifest_json])"""
    db = sqlite3.connect(path)
    db.execute(
        "CREATE TABLE onboard_proposals (id TEXT, handle TEXT, email TEXT, "
        "status TEXT, env TEXT, created_at INTEGER, manifest_json TEXT)"
    )
    db.executemany(
        "INSERT INTO onboard_proposals VALUES (?,?,?,?,?,?,?)",
        [r if len(r) == 7 else r + ("{}",) for r in rows],
    )
    db.commit()
    db.close()


def make_registry_db(path: Path, rows: list[tuple]) -> None:
    db = sqlite3.connect(path)
    db.execute(
        "CREATE TABLE slots (slot_id TEXT, assigned_to TEXT, assigned_at INTEGER, "
        "cert_minted_at INTEGER, revoked_at INTEGER)"
    )
    db.executemany("INSERT INTO slots VALUES (?,?,?,?,?)", rows)
    db.commit()
    db.close()


class ReadersTest(unittest.TestCase):
    def test_latest_proposal_per_handle_wins(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "onboard.db"
            make_onboard_db(
                p,
                [
                    ("prop-1", "alice", "alice@example.org", "failed", "zagros", 100),
                    ("prop-2", "alice", "alice@example.org", "onboarded", "zagros", 200),
                    ("prop-3", "bob", "bob@example.com", "failed", "zagros", 150),
                ],
            )
            rows = reap.read_proposals(str(p))
        by = {r["handle"]: r for r in rows}
        self.assertEqual(by["alice"]["status"], "onboarded")
        self.assertEqual(by["alice"]["id"], "prop-2")
        self.assertEqual(by["bob"]["status"], "failed")

    def test_short_tag_extracted_from_manifest(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "onboard.db"
            make_onboard_db(
                p,
                [
                    ("prop-1", "casey", "w@example.org", "onboarded", "zagros", 100,
                     json.dumps({"vendor": {"short_tag": "2nd-sig-odt-casey"}})),
                    ("prop-2", "plain", "p@example.org", "onboarded", "zagros", 100),
                ],
            )
            by = {r["handle"]: r for r in reap.read_proposals(str(p))}
        self.assertEqual(by["casey"]["short_tag"], "2nd-sig-odt-casey")
        self.assertEqual(by["plain"]["short_tag"], "")

    def test_slots_reader_skips_revoked_and_unassigned(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "registry.db"
            make_registry_db(
                p,
                [
                    ("s1", "alice", 1, 1, None),
                    ("s2", "gone", 1, 1, 999),   # revoked
                    ("s3", None, None, None, None),  # pool slot
                    ("s4", "", None, None, None),    # unassigned
                ],
            )
            rows = reap.read_slots(str(p))
        self.assertEqual([r["assigned_to"] for r in rows], ["alice"])


class ClassifyTest(unittest.TestCase):
    """One scenario per bucket, mirroring the live ZAGROS shapes (2026-07-30)."""

    def props(self):
        return [
            # healthy onboard: slot + kratos
            {"id": "p-ok", "handle": "healthy", "email": "person-41@example.org", "status": "onboarded", "env": "zagros", "created_at": 1},
            # crash-orphan: slot, proposal onboarded, NO kratos (user312x shape)
            {"id": "p-orph", "handle": "tyler", "email": "person-903@example.org", "status": "onboarded", "env": "zagros", "created_at": 2},
            # test orphan: slot + failed proposal on a test domain, no kratos
            {"id": "p-test", "handle": "livefire-9", "email": "livefire-9@example.com", "status": "failed", "env": "zagros", "created_at": 3},
            # inert residue: failed, no slot, no kratos, no netbird
            {"id": "p-res", "handle": "gate-test", "email": "gate-test@zagros.invite.local", "status": "failed", "env": "zagros", "created_at": 4},
            # failed but retried-successfully later is represented by latest row only
            # partial: failed, no slot, but a live netbird peer remains
            {"id": "p-part", "handle": "partial", "email": "person-75@example.org", "status": "failed", "env": "zagros", "created_at": 5},
        ]

    def slots(self):
        return [
            {"slot_id": "aaaa1111", "assigned_to": "healthy", "assigned_at": 1, "cert_minted_at": 1},
            {"slot_id": "bbbb2222", "assigned_to": "tyler", "assigned_at": 2, "cert_minted_at": 2},
            {"slot_id": "cccc3333", "assigned_to": "livefire-9", "assigned_at": 3, "cert_minted_at": 3},
            {"slot_id": "dddd4444", "assigned_to": "ancient", "assigned_at": 0, "cert_minted_at": 0},  # pre-console
        ]

    def kratos(self):
        return {"person-41@example.org": "active", "drill-producer@zagros.test": "active"}

    def nb(self):
        peers = [{"name": "peer-partial-zagros-2026-07-01-host.local"}]
        keys = [{"name": "peer-partial-zagros-2026-07-01", "revoked": True}]
        return peers, keys

    def run_classify(self, **over):
        peers, keys = self.nb()
        kw = dict(
            proposals=self.props(), slots=self.slots(), kratos=self.kratos(),
            nb_peers=peers, nb_keys=keys, roster_hashes={reap.email_hash("person-41@example.org")},
        )
        kw.update(over)
        return reap.classify(**kw)

    def test_crash_orphan_is_repair(self):
        plan = self.run_classify()
        orph = {o["handle"]: o for o in plan["orphans"]}
        self.assertIn("tyler", orph)
        self.assertEqual(orph["tyler"]["disposition"], "repair")
        self.assertEqual(orph["tyler"]["slot"], "bbbb2222")

    def test_test_domain_orphan_is_purge(self):
        plan = self.run_classify()
        orph = {o["handle"]: o for o in plan["orphans"]}
        self.assertEqual(orph["livefire-9"]["disposition"], "purge")

    def test_healthy_not_flagged(self):
        plan = self.run_classify()
        self.assertNotIn("healthy", {o["handle"] for o in plan["orphans"]})

    def test_inert_residue(self):
        plan = self.run_classify()
        self.assertEqual([r["handle"] for r in plan["residue"]], ["gate-test"])

    def test_failed_with_live_netbird_is_orphan_not_residue(self):
        plan = self.run_classify()
        orph = {o["handle"]: o for o in plan["orphans"]}
        self.assertIn("partial", orph)
        self.assertEqual(orph["partial"]["netbird"], ["peer:peer-partial-zagros-2026-07-01-host.local"])
        self.assertNotIn("partial", {r["handle"] for r in plan["residue"]})

    def test_revoked_setup_key_not_counted_as_live(self):
        plan = self.run_classify()
        orph = {o["handle"]: o for o in plan["orphans"]}
        self.assertTrue(all(not a.startswith("setup-key:") for a in orph["partial"]["netbird"]))

    def test_pre_console_slot_reported_not_classified(self):
        plan = self.run_classify()
        self.assertEqual([s["handle"] for s in plan["pre_console_slots"]], ["ancient"])
        self.assertNotIn("ancient", {o["handle"] for o in plan["orphans"]})

    def test_test_identity_bucket(self):
        plan = self.run_classify()
        self.assertEqual(plan["test_identities"], ["drill-producer@zagros.test"])

    def test_undeclared_excludes_rostered_and_test(self):
        kratos = dict(self.kratos())
        kratos["person-59@example.org"] = "active"
        kratos["person-44@example.org"] = "inactive"
        plan = self.run_classify(kratos=kratos)
        self.assertEqual(plan["undeclared"], ["person-59@example.org"])

    def test_no_kratos_drops_slot_orphans_keeps_netbird_evidence(self):
        plan = self.run_classify(kratos=None, roster_hashes=None)
        orph = {o["handle"]: o for o in plan["orphans"]}
        # slot-based orphan detection needs Kratos — tyler undeterminable
        self.assertNotIn("tyler", orph)
        # a failed proposal with a LIVE netbird artifact is structural evidence,
        # flagged even without Kratos, with the identity state marked unknown
        self.assertIn("partial", orph)
        self.assertEqual(orph["partial"]["kratos"], "unknown")
        self.assertIsNone(plan["test_identities"])
        self.assertIsNone(plan["undeclared"])
        self.assertIn("gate-test", {r["handle"] for r in plan["residue"]})

    def test_render_and_json_roundtrip(self):
        plan = self.run_classify()
        text = reap.render_plan(plan, "zagros", ["something degraded"])
        self.assertIn("ORPHANS (3)", text)
        self.assertIn("--revoke --purge --env zagros", text)
        self.assertIn("!! DEGRADED", text)
        json.dumps(plan)  # plan must stay JSON-serializable


class TestEmailHeuristics(unittest.TestCase):
    def test_test_domains(self):
        for e in ("x@example.com", "gate@zagros.invite.local", "d@zagros.test", "r@notional.zagros"):
            self.assertTrue(reap.is_test_email(e), e)
        for e in ("someone@partner-one.net", "a.b@partner-two.net", "x@partner-three.org"):  # oss-allow: must be non-test domains
            self.assertFalse(reap.is_test_email(e), e)

    def test_extra_domain(self):
        self.assertTrue(reap.is_test_email("x@corp.sandbox", ("@corp.sandbox",))) # oss-allow: synthetic test domain


class PeerAnchoringTest(unittest.TestCase):
    """Handle anchoring: handle `pat` must NOT
    claim `peer-pat-lee-…` / `peer-pat-kim-…` artifacts."""

    def test_handle_never_claims_longer_handles(self):
        props = [
            {"id": "p1", "handle": "pat", "email": "j@example.org", "status": "failed",
             "env": "zagros", "created_at": 1, "short_tag": ""},
        ]
        peers = [
            {"name": "peer-pat-lee-acme-pat-zagros-2026-06-14-edgebox"},
            {"name": "peer-pat-kim-pat-kim-zagros-2026-06-14-mbp.local"},
        ]
        keys = [{"name": "peer-pat-ray-pat-ray-zagros-2026-06-16", "revoked": False}]
        plan = reap.classify(props, [], kratos={}, nb_peers=peers, nb_keys=keys,
                             roster_hashes=None, env="zagros")
        # nothing attributable → the failed proposal is inert residue
        self.assertEqual([r["handle"] for r in plan["residue"]], ["pat"])
        self.assertEqual(plan["orphans"], [])

    def test_env_and_tag_forms_match(self):
        props = [
            {"id": "p1", "handle": "casey-lane", "email": "w@example.org", "status": "failed",
             "env": "zagros", "created_at": 1, "short_tag": "2nd-sig-odt-casey"},
        ]
        peers = [
            {"name": "peer-casey-lane-zagros-2026-06-18-Air.local"},           # env form
            {"name": "peer-casey-lane-2nd-sig-odt-casey-zagros-2026-06-12-x"}, # tag form
            {"name": "peer-casey-lane-othertag-zagros-2026-06-12-x"},          # wrong tag
        ]
        plan = reap.classify(props, [], kratos={}, nb_peers=peers, nb_keys=[],
                             roster_hashes=None, env="zagros")
        orph = {o["handle"]: o for o in plan["orphans"]}
        self.assertEqual(
            sorted(orph["casey-lane"]["netbird"]),
            [
                "peer:peer-casey-lane-2nd-sig-odt-casey-zagros-2026-06-12-x",
                "peer:peer-casey-lane-zagros-2026-06-18-Air.local",
            ],
        )


if __name__ == "__main__":
    unittest.main(verbosity=2)
