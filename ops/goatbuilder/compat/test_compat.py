"""The four promises of goatbuilder 1.0 are tests (docs/design/goatbuilder-v1-gate.md, group A).

Each test below is a gate row's "Proven by" made executable, against the kept
baseline of the last release (baseline.yaml names it):

  A3  no command or flag of the last release is gone without a deprecation entry
  A4  every kept `-o json` / `-o yaml` document of the last release is still
      produced, as a superset: no field removed or retyped, the same version
  A5  every GB-nnn code of the last release is still in the catalog with its
      title, unless the entry is marked retired

The unit tests build a violation of each rule and show the check refuses it.
The whole check against a build runs when GOATBUILDER_BIN names one:

  GOATBUILDER_BIN=crates/target/debug/goatbuilder python3 ops/goatbuilder/compat/test_compat.py

Needs PyYAML (pip install pyyaml).
"""
from __future__ import annotations

import copy
import importlib.util
import json
import os
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.abspath(os.path.join(HERE, "..", "..", ".."))

_spec = importlib.util.spec_from_file_location("compat", os.path.join(HERE, "compat.py"))
compat = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(compat)

REFERENCE = """# goatbuilder — command reference

## `goatbuilder`

Build and ship

**Options**

| Flag | Description | Default |
|---|---|---|
| `--output, -o <OUTPUT>` | Output format | `table` |
| `--yes` | Confirm |  |

**Subcommands**

- [`goatbuilder deploy`](#goatbuilder-deploy) — Deploy

## `goatbuilder deploy`

Put the pin onto one environment

**Arguments**

| Argument | Description |
|---|---|
| `[COMPONENT]` | The component |

**Options**

| Flag | Description | Default |
|---|---|---|
| `--env, -e <ENV>` | The environment |  |
| `--dry-run` | Print, do nothing |  |

## `goatbuilder promote graduate`

Verify the hop gates

**Arguments**

| Argument | Description |
|---|---|
| `<COMPONENT>` |  |
"""


def surface():
    return compat.parse_reference(REFERENCE)


class ReadsTheReference(unittest.TestCase):
    def test_commands_flags_aliases_and_arguments(self):
        s = surface()
        self.assertEqual(sorted(s), ["goatbuilder", "goatbuilder deploy", "goatbuilder promote graduate"])
        self.assertEqual(s["goatbuilder"]["flags"], {"--output": True, "-o": True, "--yes": False})
        self.assertEqual(s["goatbuilder deploy"]["flags"], {"--env": True, "-e": True, "--dry-run": False})
        self.assertEqual(s["goatbuilder deploy"]["args"], [{"name": "COMPONENT", "required": False}])
        self.assertEqual(s["goatbuilder promote graduate"]["args"], [{"name": "COMPONENT", "required": True}])


class A3CommandSurface(unittest.TestCase):
    def test_the_same_surface_passes(self):
        self.assertEqual(compat.surface_problems(surface(), surface(), []), [])

    def test_an_added_flag_or_command_passes(self):
        new = surface()
        new["goatbuilder deploy"]["flags"]["--wait"] = False
        new["goatbuilder federate"] = {"flags": {}, "args": []}
        self.assertEqual(compat.surface_problems(surface(), new, []), [])

    def test_a_removed_flag_is_refused(self):
        new = surface()
        del new["goatbuilder deploy"]["flags"]["--dry-run"]
        p = compat.surface_problems(surface(), new, [])
        self.assertEqual(len(p), 1, p)
        self.assertIn("goatbuilder deploy --dry-run", p[0])

    def test_a_renamed_flag_is_refused(self):
        new = surface()
        del new["goatbuilder deploy"]["flags"]["--env"]
        new["goatbuilder deploy"]["flags"]["--environment"] = True
        p = compat.surface_problems(surface(), new, [])
        self.assertEqual(len(p), 1, p)
        self.assertIn("goatbuilder deploy --env", p[0])

    def test_a_removed_short_alias_is_refused(self):
        new = surface()
        del new["goatbuilder"]["flags"]["-o"]
        self.assertIn("goatbuilder -o", compat.surface_problems(surface(), new, [])[0])

    def test_a_removed_command_is_refused(self):
        new = surface()
        del new["goatbuilder promote graduate"]
        p = compat.surface_problems(surface(), new, [])
        self.assertEqual(len(p), 1, p)
        self.assertIn("goatbuilder promote graduate", p[0])

    def test_a_flag_that_stops_taking_a_value_is_refused(self):
        new = surface()
        new["goatbuilder deploy"]["flags"]["--env"] = False
        self.assertIn("goatbuilder deploy --env", compat.surface_problems(surface(), new, [])[0])

    def test_an_optional_argument_made_required_is_refused(self):
        new = surface()
        new["goatbuilder deploy"]["args"][0]["required"] = True
        self.assertIn("goatbuilder deploy", compat.surface_problems(surface(), new, [])[0])

    def test_a_removed_argument_is_refused(self):
        new = surface()
        new["goatbuilder promote graduate"]["args"] = []
        self.assertIn("goatbuilder promote graduate", compat.surface_problems(surface(), new, [])[0])

    def test_a_deprecation_entry_names_the_removal_and_it_passes(self):
        new = surface()
        del new["goatbuilder deploy"]["flags"]["--env"]
        del new["goatbuilder promote graduate"]
        deps = [
            {"command": "goatbuilder deploy", "flag": "--env", "replacement": "--environment", "deprecated_in": "goatbuilder/v0.6.0"},
            {"command": "goatbuilder promote graduate", "replacement": "goatbuilder promote hop", "deprecated_in": "goatbuilder/v0.6.0"},
        ]
        self.assertEqual(compat.surface_problems(surface(), new, deps), [])

    def test_a_deprecation_entry_without_its_replacement_is_refused(self):
        deps = [{"command": "goatbuilder deploy", "flag": "--env", "deprecated_in": "goatbuilder/v0.6.0"}]
        self.assertTrue(compat.deprecation_problems(deps))


class A4Documents(unittest.TestCase):
    BASE = {
        "schema": "goatbuilder.output/v1",
        "verb": "status",
        "ok": False,
        "exit": 3,
        "checks": [{"name": "dev/a/portal", "ok": False, "detail": "pending"}],
    }

    def test_the_same_document_passes(self):
        self.assertEqual(compat.document_problems(self.BASE, copy.deepcopy(self.BASE)), [])

    def test_other_values_of_the_same_shape_pass(self):
        new = copy.deepcopy(self.BASE)
        new["ok"], new["exit"] = True, 0
        new["checks"] = [{"name": "x", "ok": True, "detail": "converged"}] * 3
        self.assertEqual(compat.document_problems(self.BASE, new), [])

    def test_an_added_field_passes(self):
        new = copy.deepcopy(self.BASE)
        new["inputs"] = []
        new["checks"][0]["log"] = "/var/log/x"
        self.assertEqual(compat.document_problems(self.BASE, new), [])

    def test_a_removed_field_is_refused(self):
        new = copy.deepcopy(self.BASE)
        del new["verb"]
        p = compat.document_problems(self.BASE, new)
        self.assertEqual(len(p), 1, p)
        self.assertIn("verb", p[0])

    def test_a_field_removed_inside_a_list_is_refused(self):
        new = copy.deepcopy(self.BASE)
        del new["checks"][0]["detail"]
        self.assertIn("checks[].detail", compat.document_problems(self.BASE, new)[0])

    def test_a_retyped_field_is_refused(self):
        new = copy.deepcopy(self.BASE)
        new["exit"] = "3"
        self.assertIn("exit", compat.document_problems(self.BASE, new)[0])

    def test_a_list_that_comes_back_empty_is_refused(self):
        new = copy.deepcopy(self.BASE)
        new["checks"] = []
        self.assertIn("checks", compat.document_problems(self.BASE, new)[0])

    def test_a_version_added_to_a_document_that_had_none_passes(self):
        base = {"code": "GB-008", "title": "t"}
        self.assertEqual(compat.document_problems(base, {"schema": "goat-errors.entry/v1", "code": "GB-009", "title": "u"}), [])

    def test_a_version_removed_is_refused(self):
        new = copy.deepcopy(self.BASE)
        del new["schema"]
        self.assertTrue(compat.document_problems(self.BASE, new))

    def test_a_changed_version_is_refused(self):
        new = copy.deepcopy(self.BASE)
        new["schema"] = "goatbuilder.output/v2"
        self.assertIn("schema", compat.document_problems(self.BASE, new)[0])


class A4ConditionalFields(unittest.TestCase):
    """A field a format carries only on some entries (conditional.yaml) may be absent
    from the others, and must stay on every entry its condition names."""

    def setUp(self):
        with open(os.path.join(HERE, "documents", "error-federate-undeclared.json")) as f:
            self.base = json.load(f)
        self.cond = compat.Baseline(HERE).conditional()

    def _coded(self, doc):
        """The same refusal once it has its own code: one hit of layer `code`, no score."""
        new = copy.deepcopy(doc)
        hit = copy.deepcopy(new["interpretation"]["hits"][0])
        hit.update({"code": "GB-999", "layer": "code", "evidence": "goatbuilder: GB-999 the peering is not declared", "at": 0})
        hit.pop("score")
        new["interpretation"]["hits"] = [hit]
        new["interpretation"]["unrecognised"] = False
        return new

    def test_a_refusal_that_gains_its_own_code_keeps_the_promise(self):
        self.assertEqual(compat.document_problems(self.base, self._coded(self.base), self.cond), [])

    def test_a_lexical_hit_that_loses_its_score_is_refused(self):
        new = copy.deepcopy(self.base)
        new["interpretation"]["hits"][1].pop("score")
        p = compat.document_problems(self.base, new, self.cond)
        self.assertEqual(len(p), 1, p)
        self.assertIn("interpretation.hits[1].score", p[0])
        self.assertIn("lexical", p[0])

    def test_a_field_the_list_does_not_name_is_still_refused(self):
        new = self._coded(self.base)
        new["interpretation"]["hits"][0].pop("evidence")
        p = compat.document_problems(self.base, new, self.cond)
        self.assertEqual(p, ["interpretation.hits[].evidence: the field is gone"])

    def test_a_score_that_changes_type_is_refused(self):
        new = copy.deepcopy(self.base)
        new["interpretation"]["hits"][0]["score"] = "high"
        p = compat.document_problems(self.base, new, self.cond)
        self.assertTrue(any("interpretation.hits[0].score" in x and "number" in x for x in p), p)

    def test_the_list_is_well_formed_and_names_its_reason(self):
        self.assertEqual(compat.conditional_problems(self.cond), [])
        self.assertTrue(compat.conditional_problems([{"format": "x/v1", "items": "hits[]", "field": "score", "present_when": {"layer": "lexical"}}]))
        self.assertTrue(compat.conditional_problems([{"format": "x/v1", "items": "hits", "field": "score", "present_when": {"layer": "lexical"}, "reason": "r"}]))


class A5ErrorCodes(unittest.TestCase):
    BASE = [{"code": "GB-001", "title": "unknown component"}, {"code": "GB-002", "title": "no plan"}]

    def catalog(self):
        return [
            {"code": "GB-001", "tool": "goatbuilder", "title": "unknown component"},
            {"code": "GB-002", "tool": "goatbuilder", "title": "no plan"},
        ]

    def test_the_same_codes_pass(self):
        self.assertEqual(compat.error_problems(self.BASE, self.catalog()), [])

    def test_a_new_code_passes(self):
        c = self.catalog() + [{"code": "GB-003", "tool": "goatbuilder", "title": "new"}]
        self.assertEqual(compat.error_problems(self.BASE, c), [])

    def test_an_absent_code_is_refused(self):
        p = compat.error_problems(self.BASE, self.catalog()[:1])
        self.assertEqual(len(p), 1, p)
        self.assertIn("GB-002", p[0])

    def test_a_retitled_code_is_refused(self):
        c = self.catalog()
        c[1]["title"] = "the plan file is missing"
        self.assertIn("GB-002", compat.error_problems(self.BASE, c)[0])

    def test_a_retitled_code_marked_retired_passes(self):
        c = self.catalog()
        c[1]["title"] = "retired: no plan"
        c[1]["retired"] = "goatbuilder/v0.6.0"
        self.assertEqual(compat.error_problems(self.BASE, c), [])

    def test_a_retired_code_is_still_refused_when_absent(self):
        self.assertTrue(compat.error_problems(self.BASE, self.catalog()[:1]))


class TheKeptBaseline(unittest.TestCase):
    """The baseline in this directory is whole and names one release."""

    def setUp(self):
        self.b = compat.Baseline(HERE)

    def test_it_names_the_last_release(self):
        self.assertRegex(self.b.release, r"^goatbuilder/v\d+\.\d+\.\d+$")

    def test_the_surface_is_the_release_s_whole_command_tree(self):
        s = self.b.surface()
        self.assertGreaterEqual(len(s), 30)
        self.assertIn("--output", s["goatbuilder"]["flags"])

    def test_every_kept_document_has_its_invocation(self):
        docs = self.b.documents()
        self.assertGreaterEqual(len(docs), 5)
        for d in docs:
            self.assertTrue(os.path.isfile(os.path.join(HERE, "documents", d["name"] + "." + d["format"])), d)

    def test_the_error_codes_of_the_release_are_kept(self):
        self.assertGreaterEqual(len(self.b.errors()), 100)

    def test_the_current_catalog_keeps_every_code(self):
        self.assertEqual(compat.error_problems(self.b.errors(), compat.load_catalog(REPO)), [])

    def test_the_deprecation_list_is_well_formed(self):
        self.assertEqual(compat.deprecation_problems(self.b.deprecations()), [])


class RefreshRecordsOnlyThePublishedBinary(unittest.TestCase):
    """`refresh` takes a binary only when its SHA-256 is the release's SHA256SUMS entry.

    `--version` alone proves nothing: a local build of main says the same version,
    and a wrapper around the published binary can print anything it likes.
    """

    def setUp(self):
        import tempfile
        self.tmp = tempfile.mkdtemp(prefix="compat-refresh-", dir=os.environ.get("TMPDIR"))
        self.bin = os.path.join(self.tmp, "goatbuilder-linux-amd64")
        with open(self.bin, "w") as f:
            f.write("#!/bin/sh\necho 'goatbuilder 0.5.1'\n")
        os.chmod(self.bin, 0o700)
        import hashlib
        with open(self.bin, "rb") as f:
            self.digest = hashlib.sha256(f.read()).hexdigest()
        self.root = os.path.join(self.tmp, "compat")
        os.makedirs(self.root)
        with open(os.path.join(self.root, "baseline.yaml"), "w") as f:
            f.write("release: goatbuilder/v0.5.0\n")

    def tearDown(self):
        import shutil
        shutil.rmtree(self.tmp, ignore_errors=True)

    def sums(self, digest, name="goatbuilder-linux-amd64"):
        p = os.path.join(self.tmp, "SHA256SUMS")
        with open(p, "w") as f:
            f.write(f"{'0' * 64}  goatbuilder-docs.tar.gz\n{digest}  {name}\n")
        return p

    def test_the_right_version_with_the_wrong_hash_is_refused(self):
        rc = compat.refresh(REPO, self.root, "goatbuilder/v0.5.1", self.bin, sums=self.sums("f" * 64))
        self.assertEqual(rc, 2)
        with open(os.path.join(self.root, "baseline.yaml")) as f:
            self.assertEqual(f.read(), "release: goatbuilder/v0.5.0\n")

    def test_a_hash_the_sums_give_to_another_file_is_refused(self):
        self.assertIsNotNone(compat.release_binary_problem(self.bin, self.sums(self.digest, "goatbuilder-docs.tar.gz"), "goatbuilder/v0.5.1"))

    def test_refresh_without_the_release_s_sums_is_refused(self):
        rc = compat.refresh(REPO, self.root, "goatbuilder/v0.5.1", self.bin, sums=None)
        self.assertEqual(rc, 2)

    def test_a_binary_the_sums_name_with_its_version_passes_the_gate(self):
        self.assertIsNone(compat.release_binary_problem(self.bin, self.sums(self.digest), "goatbuilder/v0.5.1"))

    def test_a_pinned_release_refuses_sums_that_disagree_with_the_pin(self):
        pins = {("goatbuilder/v0.5.1", "goatbuilder-linux-amd64"): "e" * 64}
        self.assertIsNotNone(compat.release_binary_problem(self.bin, self.sums(self.digest), "goatbuilder/v0.5.1", pins))

    def test_the_kept_baseline_is_the_pinned_published_binary(self):
        b = compat.Baseline(HERE)
        pins = compat.load_pins(HERE)
        self.assertEqual(pins[(b.release, b.meta["binary"])], b.meta["binary_sha256"])


@unittest.skipUnless(os.environ.get("GOATBUILDER_BIN"), "set GOATBUILDER_BIN to a built goatbuilder")
class TheBuildKeepsThePromise(unittest.TestCase):
    def test_every_surface_against_the_last_release(self):
        rows = compat.check_all(REPO, HERE, os.environ["GOATBUILDER_BIN"])
        failed = [r for r in rows if not r[0]]
        self.assertEqual(failed, [], json.dumps(failed, indent=1))


if __name__ == "__main__":
    unittest.main(verbosity=2)
