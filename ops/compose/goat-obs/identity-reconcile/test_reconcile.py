#!/usr/bin/env python3
"""Tests for the identity-reconcile degradation classifier.

The watcher's whole job is to not report all-clear while blind, so the pinned
behaviour is the boundary: which entries in the portal's `notes[]` mean a layer
feeding the stray verdict went dark, and which are the standing doctrinal text
a perfectly healthy portal always returns. Getting the second half wrong pins
_up=0 forever, which is how an alert gets silenced.

The note strings below are copied from the portal's own emitters —
internal/api/access.go collectAccessSources and internal/netbirdpoll
PeersForAccess.

Run: python3 -m unittest discover -s ops/compose/goat-obs/identity-reconcile
"""
import importlib.util
import json
import os
import pathlib
import tempfile
import unittest
import unittest.mock

HERE = pathlib.Path(__file__).parent
_spec = importlib.util.spec_from_file_location("reconcile", HERE / "reconcile.py")
rec = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(rec)

# Emitted unconditionally on a healthy roster — these must never blind it.
STANDING_NOTES = [
    ("writes: a peer may publish ONLY its own anchor — the router enforces "
    "`<anchor>/**` (and its `volatile/` mirror) per-UUID, deny-by-default "
    "(ADR 1025 slot-write rules), and that single-owner root is INVIOLABLE."),
    ("reads: flat community-read — every on-mesh identity may read all topics "
    "(ADR 1025); release-set membership not declared (release-sets path unset)"),
    ("reads: flat community-read is STILL in effect (ADR 1025) — the declared "
    "release-sets are the governance picture, NOT yet an enforced read partition (B2.1)"),
    'release: derived member lane "rel-them" from 3 coalition-member org(s) (ADR 1077 B2.2)',
]


class TestDegradingNotes(unittest.TestCase):
    def test_healthy_roster_is_not_degraded(self):
        self.assertEqual(rec.degrading_notes(STANDING_NOTES), [],
                         "the standing doctrinal notes must not pin _up=0 forever")

    def test_empty_notes(self):
        self.assertEqual(rec.degrading_notes([]), [])

    def test_kratos_outage_blinds(self):
        # The audit's headline case: strays=0 while the identity layer is dark.
        for note in ("kratos admin error: Get \"http://127.0.0.1:4434\": connection refused",
                     "identity layer unavailable (kratos admin not configured) — persons + "
                     "cut-off reconciliation omitted"):
            self.assertEqual(rec.degrading_notes(STANDING_NOTES + [note]), [note], note)

    def test_declared_registry_loss_blinds(self):
        # Without the declared side, cut-off and never-onboarded are the same.
        for note in ("declared registry unavailable (roster path not configured) — identity "
                     "reconciliation degrades to unknown (cut-off vs never-onboarded "
                     "indistinguishable)",
                     "declared registry load error: no such file",
                     "registry unavailable: database is locked"):
            self.assertEqual(rec.degrading_notes([note]), [note], note)

    def test_mesh_layer_is_exempt(self):
        # netbird supplies on_mesh, not the verdict — calling this blind would
        # be its own false alarm.
        for note in ("netbird mgmt slow/unreachable — mesh posture is stale "
                     "(peer snapshot 12m0s old)",
                     "netbird mgmt slow/unreachable — mesh posture omitted this call "
                     "(peers + on-mesh edges unavailable)",
                     "mesh layer unavailable (netbird mgmt URL/token not configured) — "
                     "peers + on-mesh edges omitted"):
            self.assertEqual(rec.degrading_notes([note]), [], note)

    def test_unrecognized_failure_note_blinds(self):
        # Fail-closed: a note in the portal's failure vocabulary that this
        # script has never seen still counts. Blind-but-loud beats blind-quiet.
        note = "future layer unavailable: something new broke"
        self.assertEqual(rec.degrading_notes([note]), [note])

    def test_all_degrading_notes_are_returned(self):
        notes = STANDING_NOTES + ["kratos admin error: boom",
                                  "slot aliases unavailable: disk"]
        self.assertEqual(len(rec.degrading_notes(notes)), 2)


class TestRenderProm(unittest.TestCase):
    def test_up_zero_and_reason_count_are_emitted(self):
        body = rec.render_prom("zagros", 0, 0, 0, {}, 1, 0, degraded=2)
        self.assertIn('goat_identity_reconcile_up{env="zagros"} 0', body)
        self.assertIn('goat_identity_reconcile_degraded_layers{env="zagros"} 2', body)

    def test_healthy_render(self):
        body = rec.render_prom("zagros", 1, 3, 1, {"untracked": 1}, 1, 0)
        self.assertIn('goat_identity_reconcile_up{env="zagros"} 1', body)
        self.assertIn('goat_identity_reconcile_degraded_layers{env="zagros"} 0', body)
        self.assertIn('goat_identity_strays{env="zagros",scope="unallowlisted"} 1', body)


class TestMainWritesUp(unittest.TestCase):
    """The observe target end-to-end: a roster served while Kratos is down must
    land up=0 in the .prom, not a confident zero-stray all-clear."""

    def run_main(self, payload):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        env = {"PORTAL_URL": "https://portal.invalid", "PORTAL_OPERATOR_API_KEY": "k",
               "GOAT_ENV": "zagros", "PORTAL_RESOLVE_IP": "", "ORY_SOT_GROUPS": ""}
        with unittest.mock.patch.dict(os.environ, env, clear=False), \
                unittest.mock.patch.object(rec, "fetch_roster", return_value=payload), \
                unittest.mock.patch.object(rec, "load_allowlist", return_value=[]), \
                unittest.mock.patch("sys.argv", ["reconcile.py", tmp.name]):
            rc = rec.main()
        return rc, (pathlib.Path(tmp.name) / "goat_identity_reconcile.prom").read_text()

    def test_kratos_down_reports_blind_not_healthy(self):
        rc, prom = self.run_main({
            "rows": [], "notes": STANDING_NOTES + ["kratos admin error: connection refused"]})
        self.assertIn('goat_identity_reconcile_up{env="zagros"} 0', prom)
        self.assertIn('goat_identity_reconcile_degraded_layers{env="zagros"} 1', prom)
        self.assertIn('goat_identity_strays{env="zagros",scope="all"} 0', prom)
        # Exit code stays on the stray channel: obs.yml fails its deploy leg on
        # rc=1, and a dark portal layer is not a failed deploy.
        self.assertEqual(rc, 0)

    def test_healthy_roster_reports_up(self):
        rc, prom = self.run_main({
            "rows": [{"kind": "person", "state": "out_of_band", "label": "a@b.invalid",
                      "recon_reason": "untracked"}],
            "notes": STANDING_NOTES})
        self.assertIn('goat_identity_reconcile_up{env="zagros"} 1', prom)
        self.assertIn('goat_identity_strays{env="zagros",scope="unallowlisted"} 1', prom)
        self.assertEqual(rc, 2, "an unallowlisted stray is still the louder signal")


class TestOffVocabReadsAdminMetadata(unittest.TestCase):
    """F-572 / ADR 1188: login groups live in admin-managed metadata_public. A
    groups key there, even null, marks the identity migrated. Self-editable
    traits are never read, migrated or not (GHSA-9v3v)."""

    def offvocab(self, identities):
        body = json.dumps(identities).encode()
        resp = unittest.mock.MagicMock()
        resp.__enter__.return_value.read.return_value = body
        with unittest.mock.patch.object(rec.urllib.request, "urlopen", return_value=resp):
            return rec.fetch_offvocab("http://kratos.invalid", {"PLATFORM-ADMIN"})

    def test_migrated_null_or_empty_groups_ignore_traits(self):
        up, n, bad = self.offvocab([
            {"id": "a", "traits": {"email": "a@x", "groups": ["TYPO"]}, "metadata_public": {"groups": None}},
            {"id": "b", "traits": {"email": "b@x", "groups": ["TYPO"]}, "metadata_public": {"groups": []}},
        ])
        self.assertEqual((up, n, bad), (1, 0, []))

    def test_metadata_groups_are_checked_and_legacy_traits_are_not(self):
        up, n, bad = self.offvocab([
            {"id": "m", "traits": {"email": "m@x"}, "metadata_public": {"groups": ["TYPO"]}},
            {"id": "l", "traits": {"email": "l@x", "groups": ["TYPO"]}},
            {"id": "ok", "traits": {"email": "ok@x"}, "metadata_public": {"groups": ["PLATFORM-ADMIN"]}},
        ])
        self.assertEqual((up, n, sorted(bad)), (1, 1, ["m@x"]))


if __name__ == "__main__":
    unittest.main()
