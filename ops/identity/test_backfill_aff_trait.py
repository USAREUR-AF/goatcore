"""The planner behind backfill-aff-trait.py: who gets an aff, from where, and who is left alone."""

import contextlib
import datetime
import importlib.util
import io
import json
import os
import subprocess
import sys
import tempfile
import unittest
import urllib.parse
import unittest.mock

HERE = os.path.dirname(os.path.abspath(__file__))
SPEC = importlib.util.spec_from_file_location("backfill_aff_trait", os.path.join(HERE, "backfill-aff-trait.py"))
backfill = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = backfill
SPEC.loader.exec_module(backfill)


def catalog(*rows):
    doc = {"orgs": list(rows)}
    with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as f:
        json.dump(doc, f)
    try:
        return backfill.load_catalog(f.name)
    finally:
        os.unlink(f.name)


ACME = {"id": "acme", "kind": "vendor", "nation": "USA", "status": "active", "aliases": ["acme-corp"]}
UNIT = {"id": "some-unit", "kind": "organic", "nation": "USA", "status": "active"}
GONE = {"id": "old-org", "kind": "vendor", "nation": "USA", "status": "absorbed"}


def identity(email, short_tag=None, aff=None, meta=None, self_edited=False):
    """An identity whose org/aff sit in admin-managed metadata_public, or in
    self-editable traits when self_edited (never read since GHSA-9v3v)."""
    traits = {"email": email}
    authz = traits if self_edited else {}
    if short_tag is not None:
        authz["org"] = {"short_tag": short_tag, "display_name": short_tag.title()}
    if aff is not None:
        authz["aff"] = aff
    out = {"id": f"id-{email}", "traits": traits}
    if meta is not None or (authz and not self_edited):
        out["metadata_public"] = {**(authz if not self_edited else {}), **(meta or {})}
    return out


class CatalogTest(unittest.TestCase):
    def test_ids_and_aliases_resolve_to_the_same_row(self):
        cat = catalog(ACME)
        self.assertIs(cat["acme"], cat["acme-corp"])

    def test_an_empty_catalog_is_refused(self):
        with self.assertRaises(SystemExit):
            catalog()

    def test_ids_override_aliases_and_first_alias_wins_in_both_formats(self):
        import yaml

        rows = [{"id": "acme", "aliases": [" unit ", "shared"]},
                {"id": "unit", "aliases": ["shared"]}]
        for ordered in (rows, list(reversed(rows))):
            for suffix in (".json", ".yaml"):
                with self.subTest(order=ordered, suffix=suffix), tempfile.TemporaryDirectory() as tmp:
                    path = os.path.join(tmp, "catalog" + suffix)
                    doc = {"orgs": ordered}
                    with open(path, "w") as source:
                        source.write(json.dumps(doc) if suffix == ".json" else yaml.safe_dump(doc))
                    cat = backfill.load_catalog(path)
                    self.assertEqual(cat["unit"]["id"], "unit")
                    self.assertEqual(cat["shared"]["id"], ordered[0]["id"])

    def test_aff_value_carries_id_kind_and_nation(self):
        aff, why = backfill.aff_for(ACME)
        self.assertIsNone(why)
        self.assertEqual(aff, {"id": "acme", "kind": "vendor", "nation": "USA"})

    def test_a_row_without_a_nation_omits_it(self):
        aff, _ = backfill.aff_for({"id": "x", "kind": "civil", "status": "active"})
        self.assertEqual(aff, {"id": "x", "kind": "civil"})

    def test_a_kind_the_schema_rejects_is_refused(self):
        aff, why = backfill.aff_for({"id": "x", "kind": "commercial", "nation": "USA", "status": "active"})
        self.assertIsNone(aff)
        self.assertIn("kind", why)


class PlanTest(unittest.TestCase):
    def test_short_tag_resolves_by_id(self):
        actions, skipped = backfill.plan([identity("a@x.test", "acme")], catalog(ACME))
        self.assertEqual(skipped, [])
        self.assertEqual(actions[0]["aff"], {"id": "acme", "kind": "vendor", "nation": "USA"})
        self.assertEqual(actions[0]["source"], "org.short_tag")

    def test_short_tag_resolves_by_alias_to_the_catalog_id(self):
        actions, _ = backfill.plan([identity("a@x.test", "acme-corp")], catalog(ACME))
        self.assertEqual(actions[0]["aff"]["id"], "acme")

    def test_an_identity_already_carrying_aff_is_left_alone(self):
        rows = [identity("a@x.test", "acme", aff={"id": "acme", "kind": "vendor"})]
        actions, skipped = backfill.plan(rows, catalog(ACME))
        self.assertEqual(actions, [])
        self.assertIn("already carries aff 'acme'", skipped[0]["why"])

    def test_a_disagreeing_aff_is_reported_not_changed(self):
        rows = [identity("a@x.test", "some-unit", aff={"id": "acme", "kind": "vendor"})]
        actions, skipped = backfill.plan(rows, catalog(ACME, UNIT))
        self.assertEqual(actions, [])
        self.assertIn("forward-only", skipped[0]["why"])

    def test_an_empty_aff_object_counts_as_absent_and_is_filled(self):
        actions, skipped = backfill.plan([identity("a@x.test", "acme", aff={})], catalog(ACME))
        self.assertEqual(len(actions), 1)
        self.assertEqual(skipped, [])

    def test_no_short_tag_is_skipped_with_the_remedy(self):
        actions, skipped = backfill.plan([identity("a@x.test")], catalog(ACME))
        self.assertEqual(actions, [])
        self.assertIn("--assign", skipped[0]["why"])

    def test_a_tag_the_catalog_does_not_know_is_skipped(self):
        actions, skipped = backfill.plan([identity("a@x.test", "a-person-slug")], catalog(ACME))
        self.assertEqual(actions, [])
        self.assertIn("not in the catalog", skipped[0]["why"])

    def test_an_absorbed_org_is_not_written(self):
        actions, skipped = backfill.plan([identity("a@x.test", "old-org")], catalog(ACME, GONE))
        self.assertEqual(actions, [])
        self.assertIn("absorbed", skipped[0]["why"])

    def test_assign_overrides_the_tag_and_is_recorded_as_the_source(self):
        rows = [identity("a@x.test", "a-person-slug")]
        actions, skipped = backfill.plan(rows, catalog(ACME), assign={"a@x.test": "acme"})
        self.assertEqual(skipped, [])
        self.assertEqual(actions[0]["aff"]["id"], "acme")
        self.assertEqual(actions[0]["source"], "assign")

    def test_only_limits_the_run_to_one_email_case_insensitively(self):
        rows = [identity("a@x.test", "acme"), identity("b@x.test", "acme")]
        actions, skipped = backfill.plan(rows, catalog(ACME), only="A@X.TEST")
        self.assertEqual([a["email"] for a in actions], ["a@x.test"])
        self.assertEqual(skipped, [])


class MetadataTest(unittest.TestCase):
    """aff is an authorization field: it is read the way every reader reads it
    and written to metadata_public, never to self-editable traits."""

    def test_the_write_goes_to_metadata_public_with_its_other_keys_kept(self):
        row = identity("a@x.test", meta={"org": {"short_tag": "acme"}, "groups": ["DEVELOPER"]})
        actions, _ = backfill.plan([row], catalog(ACME))
        seen = []
        with unittest.mock.patch.object(backfill, "_req", lambda method, url, body=None: (seen.append(body), ({}, {}))[1]):
            backfill.patch_aff("http://kratos.test", actions[0])
        self.assertEqual(seen, [[{"op": "add", "path": "/metadata_public", "value": {
            "org": {"short_tag": "acme"}, "groups": ["DEVELOPER"],
            "aff": {"id": "acme", "kind": "vendor", "nation": "USA"}}}]])

    def test_an_unmigrated_identity_gets_no_groups_marker(self):
        actions, _ = backfill.plan([identity("a@x.test", "acme")], catalog(ACME))
        self.assertNotIn("groups", actions[0]["metadata_public"])
        self.assertEqual(actions[0]["metadata_public"]["aff"], {"id": "acme", "kind": "vendor", "nation": "USA"})

    def test_a_migrated_identity_ignores_an_aff_in_its_traits(self):
        row = identity("a@x.test", aff={"id": "self-granted", "kind": "vendor"},
                       meta={"groups": [], "org": {"short_tag": "acme"}}, self_edited=True)
        actions, skipped = backfill.plan([row], catalog(ACME))
        self.assertEqual((len(actions), skipped), (1, []))
        self.assertEqual(actions[0]["aff"]["id"], "acme")

    def test_an_unmigrated_identity_ignores_a_self_edited_org(self):
        # GHSA-9v3v: traits.org is self-editable; it must not pick the org
        # whose aff becomes admin-managed metadata_public.aff.
        row = identity("a@x.test", "acme", self_edited=True)
        actions, skipped = backfill.plan([row], catalog(ACME))
        self.assertEqual(actions, [])
        self.assertIn("no org.short_tag", skipped[0]["why"])

    def test_an_unmigrated_identity_with_a_self_edited_aff_is_still_fillable(self):
        row = identity("a@x.test", aff={"id": "self-granted", "kind": "vendor"},
                       meta={"org": {"short_tag": "acme"}}, self_edited=True)
        actions, skipped = backfill.plan([row], catalog(ACME))
        self.assertEqual((len(actions), skipped), (1, []))
        self.assertEqual(actions[0]["aff"]["id"], "acme")

    def test_a_migrated_identity_ignores_an_org_in_its_traits(self):
        row = identity("a@x.test", "acme", meta={"groups": []}, self_edited=True)
        actions, skipped = backfill.plan([row], catalog(ACME))
        self.assertEqual(actions, [])
        self.assertIn("no org.short_tag", skipped[0]["why"])

    def test_an_aff_already_in_metadata_public_is_left_alone(self):
        row = identity("a@x.test", "acme", meta={"aff": {"id": "acme", "kind": "vendor"}})
        actions, skipped = backfill.plan([row], catalog(ACME))
        self.assertEqual(actions, [])
        self.assertIn("already carries aff 'acme'", skipped[0]["why"])


class VerifyTest(unittest.TestCase):
    def test_without_a_catalog_every_missing_identity_is_listed_unsplit(self):
        rows = [
            identity("a@x.test", "acme", aff={"id": "acme", "kind": "vendor"}),
            identity("b@x.test", "acme", aff={}),
            identity("c@x.test"),
        ]
        fillable, unresolvable = backfill.verify(rows)
        self.assertEqual(fillable, ["b@x.test", "c@x.test"])
        self.assertEqual(unresolvable, [])

    def test_a_catalog_splits_the_fillable_from_the_unresolvable(self):
        rows = [
            identity("a@x.test", "acme", aff={"id": "acme", "kind": "vendor"}),
            identity("b@x.test", "acme", aff={}),
            identity("c@x.test"),
            identity("d@x.test", "a-person-slug"),
            identity("e@x.test", "old-org"),
        ]
        fillable, unresolvable = backfill.verify(rows, catalog(ACME, GONE))
        self.assertEqual(fillable, ["b@x.test"])
        self.assertEqual([email for email, _ in unresolvable], ["c@x.test", "d@x.test", "e@x.test"])
        self.assertIn("--assign", dict(unresolvable)["c@x.test"])
        self.assertIn("absorbed", dict(unresolvable)["e@x.test"])

    # The gate reads "nothing fillable is left", so a population whose only
    # gaps are unresolvable passes: --assign or a catalog fix is the operator's
    # call, and counting those would keep the gate red whatever the run did.
    def test_only_unresolvable_gaps_leave_nothing_fillable(self):
        fillable, unresolvable = backfill.verify([identity("c@x.test")], catalog(ACME))
        self.assertEqual(fillable, [])
        self.assertEqual(len(unresolvable), 1)

    def test_assign_moves_an_identity_into_the_fillable_count(self):
        rows = [identity("d@x.test", "a-person-slug")]
        fillable, unresolvable = backfill.verify(rows, catalog(ACME), assign={"d@x.test": "acme"})
        self.assertEqual(fillable, ["d@x.test"])
        self.assertEqual(unresolvable, [])


class ArgsTest(unittest.TestCase):
    def test_assign_wants_email_equals_org(self):
        self.assertEqual(backfill.parse_assignments(["A@X.test=acme"]), {"a@x.test": "acme"})
        with self.assertRaises(SystemExit):
            backfill.parse_assignments(["a@x.test"])

    def test_next_link_reads_the_kratos_link_header(self):
        headers = {"Link": '</admin/identities?page_size=500&page_token=abc>; rel="next"'}
        self.assertEqual(backfill._next_link(headers), "/admin/identities?page_size=500&page_token=abc")
        self.assertIsNone(backfill._next_link({}))


class FakeKratos:
    """Stands in for _req. GETs are served from `pages`, keyed by page_token
    ('' for the first page) as (identities, next token or None); PATCH returns
    what `stored` makes of the identity id and the patch body. Any other read
    fails the test, so a second GET per identity cannot hide."""

    def __init__(self, pages, stored=None):
        self.pages, self.stored, self.calls = pages, stored, []

    def __call__(self, method, url, body=None):
        self.calls.append((method, url))
        parsed = urllib.parse.urlparse(url)
        if method == "PATCH":
            return self.stored(parsed.path.rsplit("/", 1)[-1], body), {}
        if parsed.path != "/admin/identities":
            raise AssertionError(f"unexpected {method} {url}")
        batch, nxt = self.pages[urllib.parse.parse_qs(parsed.query).get("page_token", [""])[0]]
        return batch, ({"Link": f'</admin/identities?page_size=500&page_token={nxt}>; rel="next"'} if nxt else {})


def stored_with_body(identity_id, body):
    """What Kratos returns after the JSON Patch: the identity carrying the new metadata_public."""
    return {"id": identity_id, "traits": {}, "metadata_public": body[0]["value"]}


def run_main(fake, *argv):
    """(exit code, stdout, stderr) of main() against a fake Kratos."""
    out, err = io.StringIO(), io.StringIO()
    with unittest.mock.patch.object(backfill, "_req", fake), contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        try:
            code = backfill.main(["--admin", "http://kratos.test", *argv])
        except SystemExit as exc:
            code = exc.code
    return code, out.getvalue(), err.getvalue()


class MainTest(unittest.TestCase):
    def setUp(self):
        with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as f:
            json.dump({"orgs": [ACME]}, f)
        self.catalog = f.name
        self.addCleanup(os.unlink, f.name)


class PagingTest(MainTest):
    """The Link header alone ends the listing."""

    def test_a_short_page_with_a_next_link_is_followed_to_the_end(self):
        fake = FakeKratos({
            "": ([identity("a@x.test"), identity("b@x.test")], "t1"),
            "t1": ([identity("c@x.test")], "t2"),
            "t2": ([identity("d@x.test")], None),
        })
        with unittest.mock.patch.object(backfill, "_req", fake):
            listed = backfill.list_identities("http://kratos.test")
        self.assertEqual([i["traits"]["email"] for i in listed], ["a@x.test", "b@x.test", "c@x.test", "d@x.test"])

    def test_a_page_token_served_twice_is_an_error_not_a_stop(self):
        fake = FakeKratos({"": ([identity("a@x.test")], "t1"), "t1": ([identity("b@x.test")], "t1")})
        with unittest.mock.patch.object(backfill, "_req", fake), self.assertRaises(ValueError):
            backfill.list_identities("http://kratos.test")

    def test_verify_counts_an_identity_past_a_short_page(self):
        fake = FakeKratos({
            "": ([identity("a@x.test", aff={"id": "acme", "kind": "vendor"})], "t1"),
            "t1": ([identity("b@x.test", "acme")], None),
        })
        code, out, _ = run_main(fake, "--catalog", self.catalog, "--verify")
        self.assertEqual(code, 1)
        self.assertIn("missing  b@x.test", out)


class EnvCatalogTest(MainTest):
    """--env reads YAML with PyYAML or a JSON export without it."""

    def test_env_prefers_the_json_beside_the_yaml(self):
        with tempfile.TemporaryDirectory() as tmp, unittest.mock.patch.object(backfill, "HERE", tmp):
            os.mkdir(os.path.join(tmp, "orgs"))
            yaml_path = os.path.join(tmp, "orgs", "zagros.yaml")
            self.assertEqual(backfill.catalog_path(None, "zagros"), yaml_path)
            open(os.path.join(tmp, "orgs", "zagros.json"), "w").close()
            self.assertEqual(backfill.catalog_path(None, "zagros"), os.path.join(tmp, "orgs", "zagros.json"))
        self.assertEqual(backfill.catalog_path("given.json", "zagros"), "given.json")

    @unittest.skipUnless(os.path.isfile(os.path.join(backfill.HERE, "orgs", "zagros.yaml")), "no org catalog in this tree")
    def test_env_reads_the_committed_yaml_with_pyyaml(self):
        fake = FakeKratos({"": ([identity("a@x.test", aff={"id": "acme", "kind": "vendor"})], None)})
        code, out, err = run_main(fake, "--env", "zagros", "--verify")
        self.assertEqual((code, err), (0, ""))
        self.assertIn(os.path.join("orgs", "zagros.yaml"), out)
        self.assertIn("1 identities; 0 fillable", out)

    @unittest.skipUnless(os.path.isfile(os.path.join(backfill.HERE, "orgs", "zagros.yaml")), "no org catalog in this tree")
    def test_missing_pyyaml_refuses_yaml_before_reading_kratos(self):
        for mode in ("--verify", "--apply"):
            with self.subTest(mode=mode), unittest.mock.patch.dict(sys.modules, {"yaml": None}):
                fake = FakeKratos({})
                code, _, _ = run_main(fake, "--env", "zagros", mode)
            self.assertIn("pass --catalog orgs/zagros.json", str(code))
            self.assertEqual(fake.calls, [])

    def test_json_works_without_pyyaml(self):
        fake = FakeKratos({"": ([identity("a@x.test", "acme")], None)})
        with unittest.mock.patch.dict(sys.modules, {"yaml": None}):
            code, out, err = run_main(fake, "--catalog", self.catalog, "--verify")
        self.assertEqual((code, err), (1, ""))
        self.assertIn("1 fillable but missing aff.id", out)

    @unittest.skipUnless(os.path.isfile(os.path.join(backfill.HERE, "orgs", "zagros.yaml")), "no org catalog in this tree")
    def test_standalone_script_reports_missing_pyyaml_on_stderr(self):
        result = subprocess.run(
            [sys.executable, "-S", os.path.join(HERE, "backfill-aff-trait.py"),
             "--env", "zagros", "--verify"], capture_output=True, text=True, check=False)
        self.assertEqual(result.returncode, 1)
        self.assertIn("pass --catalog orgs/zagros.json", result.stderr)
        self.assertNotIn("Traceback", result.stderr)


class YamlCatalogTest(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.path = os.path.join(tmp.name, "test.yaml")

    def write(self, text):
        with open(self.path, "w", encoding="utf-8") as source:
            source.write(text)

    def test_comments_and_trailing_spaces_do_not_change_values(self):
        self.write("orgs:\n- id: acme\n  display_name: Acme # comment\n  kind: vendor   \n"
                   "  status: active\n  aliases: [acme-corp]\n")
        cat = backfill.load_catalog(self.path)
        self.assertEqual(cat["acme"]["display_name"], "Acme")
        self.assertEqual(cat["acme"]["kind"], "vendor")
        self.assertIs(cat["acme"], cat["acme-corp"])
        self.assertEqual(backfill.aff_for(cat["acme"]), ({"id": "acme", "kind": "vendor"}, None))

    def test_scalar_types_follow_yaml_and_quoted_scalars_remain_strings(self):
        for scalar, expected in (("yes", True), ("null", None), ("007", 7),
                                 ("2026-01-01", datetime.date(2026, 1, 1))):
            with self.subTest(scalar=scalar):
                self.write(f"orgs:\n- id: acme\n  display_name: {scalar}\n")
                actual = backfill.load_catalog(self.path)["acme"]["display_name"]
                self.assertEqual(actual, expected)
                self.assertIs(type(actual), type(expected))
                self.write(f"orgs:\n- id: acme\n  display_name: '{scalar}'\n")
                self.assertEqual(backfill.load_catalog(self.path)["acme"]["display_name"], scalar)

    def test_typed_ids_use_the_same_affiliation_value_for_conflict_checks(self):
        for scalar in ("123", "2026-01-01"):
            self.write(f"orgs:\n- id: {scalar}\n  kind: vendor\n  status: active\n  aliases: [lookup]\n")
            cat = backfill.load_catalog(self.path)
            for tag in (scalar, "lookup"):
                with self.subTest(scalar=scalar, tag=tag):
                    row = identity("a@x.test", meta={"org": {"short_tag": tag},
                                   "aff": {"id": scalar, "kind": "vendor"}})
                    actions, skipped = backfill.plan([row], cat)
                    self.assertEqual(actions, [])
                    self.assertEqual(skipped[0]["why"], f"already carries aff {scalar!r}")
                    row["metadata_public"]["aff"]["id"] = "other"
                    actions, skipped = backfill.plan([row], cat)
                    self.assertEqual(actions, [])
                    self.assertIn("forward-only", skipped[0]["why"])
                    del row["metadata_public"]["aff"]
                    actions, skipped = backfill.plan([row], cat)
                    self.assertEqual(skipped, [])
                    self.assertEqual(actions[0]["aff"]["id"], scalar)

    def test_invalid_scalars_report_catalog_error_before_kratos(self):
        for value in ("2026-13-01", "2026-02-30", "!!timestamp garbage", "!!bool garbage",
                      "!!int ''", "!!float ''"):
            self.write(f"orgs:\n- id: acme\n  display_name: {value}\n")
            for mode in ([], ["--verify"], ["--apply"]):
                with self.subTest(value=value, mode=mode):
                    fake = FakeKratos({})
                    code, _, _ = run_main(fake, "--catalog", self.path, *mode)
                    self.assertIn(f"{self.path}: invalid YAML catalog", str(code))
                    self.assertEqual(fake.calls, [])
            result = subprocess.run(
                [sys.executable, os.path.join(HERE, "backfill-aff-trait.py"),
                 "--catalog", self.path, "--verify"], capture_output=True, text=True, check=False)
            self.assertEqual(result.returncode, 1)
            self.assertIn(f"{self.path}: invalid YAML catalog", result.stderr)
            self.assertNotIn("Traceback", result.stderr)

    def test_bad_catalogs_fail_before_kratos_in_preview_verify_and_apply(self):
        texts = ["orgs:\n- id: acme\n  display_name: Acme: Company\n", "", "[]", "{}",
                 "orgs: {}", "orgs: [false]", "orgs: []", "orgs: [{id: acme, aliases: alias}]",
                 "!!python/object/apply:os.system ['false']"]
        for text in texts:
            self.write(text)
            for mode in ([], ["--verify"], ["--apply"]):
                with self.subTest(text=text, mode=mode):
                    fake = FakeKratos({})
                    code, _, _ = run_main(fake, "--catalog", self.path, *mode)
                    self.assertNotEqual(code, 0)
                    self.assertIn(self.path, str(code))
                    self.assertEqual(fake.calls, [])

    def test_unreadable_and_invalid_json_catalogs_fail_before_kratos(self):
        fake = FakeKratos({})
        code, _, _ = run_main(fake, "--catalog", self.path, "--apply")
        self.assertIn(self.path, str(code))
        self.assertEqual(fake.calls, [])
        self.path = os.path.splitext(self.path)[0] + ".json"
        self.write("{")
        code, _, _ = run_main(fake, "--catalog", self.path, "--apply")
        self.assertIn("invalid JSON catalog", str(code))
        self.assertEqual(fake.calls, [])


class AssignRefusalTest(MainTest):
    """An --assign the catalog lacks is refused the same way under --verify as
    under --apply, before Kratos is read."""

    def assert_refused(self, *argv):
        fake = FakeKratos({"": ([identity("a@x.test", "a-person-slug")], None)})
        code, _, err = run_main(fake, *argv)
        self.assertEqual(code, 2)
        self.assertEqual(fake.calls, [])
        return err

    def test_verify_refuses_an_org_the_catalog_lacks(self):
        err = self.assert_refused("--catalog", self.catalog, "--assign", "a@x.test=nope", "--verify")
        self.assertIn("a@x.test=nope: not in the catalog", err)

    def test_apply_refuses_it_the_same_way(self):
        err = self.assert_refused("--catalog", self.catalog, "--assign", "a@x.test=nope", "--apply")
        self.assertIn("a@x.test=nope: not in the catalog", err)

    def test_verify_without_a_catalog_cannot_check_an_assign_and_refuses_it(self):
        err = self.assert_refused("--assign", "a@x.test=acme", "--verify")
        self.assertIn("--assign needs --catalog or --env", err)

    def test_a_known_org_is_accepted_and_counted_fillable(self):
        fake = FakeKratos({"": ([identity("a@x.test", "a-person-slug")], None)})
        code, out, _ = run_main(fake, "--catalog", self.catalog, "--assign", "a@x.test=acme-corp", "--verify")
        self.assertEqual(code, 1)
        self.assertIn("missing  a@x.test", out)


class ApplyTest(MainTest):
    """The PATCH response is the check: one round trip per identity."""

    def test_apply_checks_the_patch_response_without_a_second_get(self):
        fake = FakeKratos({"": ([identity("a@x.test", "acme"), identity("b@x.test", "acme")], None)},
                          stored=stored_with_body)
        code, out, err = run_main(fake, "--catalog", self.catalog, "--apply")
        self.assertEqual((code, err), (0, ""))
        self.assertEqual([method for method, _ in fake.calls], ["GET", "PATCH", "PATCH"])
        self.assertIn("2 set, 0 failed, 0 skipped", out)

    def test_a_patch_response_without_the_aff_counts_as_failed(self):
        fake = FakeKratos({"": ([identity("a@x.test", "acme")], None)},
                          stored=lambda identity_id, body: {"id": identity_id, "traits": {}})
        code, out, err = run_main(fake, "--catalog", self.catalog, "--apply")
        self.assertEqual(code, 2)
        self.assertIn("carries aff.id None", err)
        self.assertIn("0 set, 1 failed", out)


if __name__ == "__main__":
    unittest.main()
