"""GHSA-9v3v: the authz backfill must not launder a self-claimed traits value
(groups, aff, org, vendor) into admin-managed metadata_public. Only a trusted
record keyed by Kratos identity id can back a promotion."""
import contextlib
import importlib.util
import io
import json
import pathlib
import sys
import tempfile
import unittest
import unittest.mock

_spec = importlib.util.spec_from_file_location(
    "backfill_authz", pathlib.Path(__file__).with_name("backfill-authz-traits-to-metadata.py"))
bf = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(bf)

AFF = {"id": "usareur-g6", "kind": "organic", "nation": "US"}
ORG = {"short_tag": "usareur-g6", "display_name": "G6"}
VENDOR = {"short_tag": "trusted-vendor"}


def _ident(ident, email, traits_groups=None, meta=None, **authz):
    traits = {"email": email, "name": "x"}
    if traits_groups is not None:
        traits["groups"] = traits_groups
    traits.update(authz)
    i = {"id": ident, "traits": traits}
    if meta is not None:
        i["metadata_public"] = meta
    return i


def _plan(ids, trusted=None):
    return bf.plan(ids, "traits", "metadata_public", forward=True, trusted=trusted)


class WithoutTrustedRecord(unittest.TestCase):
    def test_self_granted_groups_are_not_promoted(self):
        actions, _ = _plan([_ident("i1", "u@example.test", ["PLATFORM-ADMIN"])])
        self.assertEqual(actions[0]["dst"]["groups"], [])
        self.assertEqual(actions[0]["needs_trust"], ["groups"])
        self.assertEqual(actions[0]["promoted"], {})

    def test_metadata_without_groups_key_is_not_treated_as_trusted(self):
        ids = [_ident("i1", "u@example.test", ["PLATFORM-ADMIN"], meta={"external_subjects": []})]
        actions, _ = _plan(ids)
        self.assertEqual(actions[0]["dst"]["groups"], [])
        self.assertEqual(actions[0]["needs_trust"], ["groups"])

    def test_self_claimed_aff_org_vendor_are_not_promoted(self):
        ids = [_ident("i1", "u@example.test", aff=AFF, org=ORG, vendor=VENDOR)]
        actions, _ = _plan(ids)
        for k in ("aff", "org", "vendor"):
            self.assertNotIn(k, actions[0]["dst"])
        self.assertEqual(sorted(actions[0]["needs_trust"]), ["aff", "org", "vendor"])

    def test_empty_values_need_no_record(self):
        actions, _ = _plan([_ident("i1", "u@example.test", [], aff={})])
        self.assertEqual(actions[0]["needs_trust"], [])
        self.assertEqual(actions[0]["dst"], {"groups": []})


class WithTrustedRecord(unittest.TestCase):
    def test_trusted_record_drops_self_granted_groups(self):
        ids = [_ident("i1", "Self@Example.Test", ["OBSERVER", "PLATFORM-ADMIN"])]
        actions, _ = _plan(ids, trusted={"i1": {"email": "self@example.test", "groups": ["OBSERVER"]}})
        self.assertEqual(actions[0]["dst"]["groups"], ["OBSERVER"])
        self.assertIn(("groups", ["PLATFORM-ADMIN"], "not in trusted record"), actions[0]["dropped"])
        self.assertIn("groups", actions[0]["remove"])

    def test_identity_absent_from_trusted_record_promotes_nothing(self):
        ids = [_ident("i1", "nobody@example.test", ["PLATFORM-ADMIN"], aff=AFF)]
        actions, _ = _plan(ids, trusted={})
        self.assertEqual(actions[0]["dst"], {"groups": []})
        self.assertEqual({d[0] for d in actions[0]["dropped"]}, {"groups", "aff"})

    def test_migrated_identity_never_promotes(self):
        ids = [_ident("i1", "m@example.test", ["PLATFORM-ADMIN"], meta={"groups": []})]
        actions, _ = _plan(ids, trusted={"i1": {"groups": ["PLATFORM-ADMIN"]}})
        self.assertEqual(actions[0]["dst"]["groups"], [])

    def test_aff_org_vendor_promoted_only_when_equal_to_record(self):
        ids = [_ident("i1", "u@example.test", ["OBSERVER"], aff=AFF, org=ORG, vendor=VENDOR)]
        rec = {"i1": {"groups": ["OBSERVER"], "aff": AFF, "org": {"short_tag": "other", "display_name": "O"}}}
        actions, _ = _plan(ids, trusted=rec)
        dst = actions[0]["dst"]
        self.assertEqual(dst["aff"], AFF)
        self.assertNotIn("org", dst)
        self.assertNotIn("vendor", dst)
        self.assertEqual({d[0] for d in actions[0]["dropped"]}, {"org", "vendor"})
        self.assertEqual(sorted(actions[0]["remove"]), ["aff", "groups", "org", "vendor"])

    def test_record_is_keyed_by_identity_id_not_email(self):
        # The attacker sets traits.email to an address the record lists; the
        # record is keyed by id, so the attacker's id backs nothing.
        ids = [_ident("attacker", "departed-admin@example.test", ["PLATFORM-ADMIN", "IAM-ADMIN"])]
        rec = {"departed-admin-id": {"email": "departed-admin@example.test", "groups": ["PLATFORM-ADMIN", "IAM-ADMIN"]}}
        actions, _ = _plan(ids, trusted=rec)
        self.assertEqual(actions[0]["dst"]["groups"], [])

    def test_secondary_email_must_agree_with_identity(self):
        ids = [_ident("i1", "changed@example.test", ["OBSERVER"])]
        actions, _ = _plan(ids, trusted={"i1": {"email": "orig@example.test", "groups": ["OBSERVER"]}})
        self.assertEqual(actions[0]["dst"]["groups"], [])
        self.assertIn("disagrees", actions[0]["dropped"][0][2])

    def test_non_string_group_items_are_dropped_and_reported(self):
        ids = [_ident("i1", "u@example.test", [{"a": 1}, ["x"], 7, "OBSERVER"])]
        actions, _ = _plan(ids, trusted={"i1": {"groups": ["OBSERVER"]}})
        self.assertEqual(actions[0]["dst"]["groups"], ["OBSERVER"])
        self.assertIn(("groups", [{"a": 1}, ["x"], 7], "not a group name (non-string item)"),
                      actions[0]["dropped"])

    def test_email_keyed_record_is_refused(self):
        with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as f:
            json.dump({"u@example.test": ["PLATFORM-ADMIN"]}, f)
        with self.assertRaises(SystemExit):
            bf.load_trusted(f.name)
        pathlib.Path(f.name).unlink()


class ApplyGate(unittest.TestCase):
    """main(): --apply without --trusted writes nothing while any identity
    would promote a self-claimed value."""

    def _main(self, ids, *argv):
        writes = []
        with unittest.mock.patch.object(bf, "list_identities", return_value=ids), \
             unittest.mock.patch.object(bf, "apply_action", lambda *a: writes.append(a)), \
             unittest.mock.patch.object(sys, "argv", ["backfill", *argv]), \
             contextlib.redirect_stdout(io.StringIO()) as out, \
             contextlib.redirect_stderr(io.StringIO()):
            rc = bf.main()
        return rc, writes, out.getvalue()

    def test_apply_without_trusted_refuses_self_granted_groups(self):
        rc, writes, out = self._main([_ident("i1", "u@example.test", ["PLATFORM-ADMIN"])], "--apply")
        self.assertEqual((rc, writes), (3, []))
        self.assertIn('NEEDS-TRUSTED i1 u@example.test groups=["PLATFORM-ADMIN"]', out)

    def test_apply_without_trusted_refuses_self_claimed_aff(self):
        rc, writes, _ = self._main([_ident("i1", "u@example.test", aff=AFF)], "--apply")
        self.assertEqual((rc, writes), (3, []))

    def test_apply_without_trusted_proceeds_when_nothing_is_promoted(self):
        rc, writes, _ = self._main([_ident("i1", "u@example.test", [])], "--apply")
        self.assertEqual((rc, len(writes)), (0, 1))

    def test_apply_with_trusted_reports_drops(self):
        with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as f:
            json.dump({"i1": {"groups": ["OBSERVER"]}}, f)
        ids = [_ident("i1", "u@example.test", ["OBSERVER", "PLATFORM-ADMIN"], aff=AFF)]
        rc, writes, out = self._main(ids, "--apply", "--trusted", f.name)
        pathlib.Path(f.name).unlink()
        self.assertEqual((rc, len(writes)), (0, 1))
        self.assertEqual(writes[0][1]["dst"], {"groups": ["OBSERVER"]})
        self.assertIn('PROMOTE groups=["OBSERVER"]', out)
        self.assertIn('DROP groups=["PLATFORM-ADMIN"]', out)
        self.assertIn('DROP aff=', out)


if __name__ == "__main__":
    unittest.main()
