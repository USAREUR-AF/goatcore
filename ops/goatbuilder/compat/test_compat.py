"""The four promises of goatbuilder 1.0 are tests (docs/design/goatbuilder-v1-gate.md, group A).

Each test below is a gate row's "Proven by" made executable, against the kept
baseline of the last release (baseline.yaml names it):

  A3  no command or flag of the last release is gone without a deprecation entry
  A4  every kept `-o json` / `-o yaml` document of the last release is still
      produced, as a superset: no field removed or retyped, the same version

The unit tests build a violation of each rule and show the check refuses it.
The whole check against a build runs when GOATBUILDER_BIN names one:

  GOATBUILDER_BIN=crates/target/debug/goatbuilder python3 ops/goatbuilder/compat/test_compat.py

Needs PyYAML (pip install pyyaml).
"""
from __future__ import annotations

import copy
import importlib.util
import json
import re
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


HELP = """Print a shell completion script

Usage: goatbuilder completion [OPTIONS] <SHELL>

Arguments:
  <SHELL>  The shell to generate for [possible values: bash, elvish, fish, powershell,
           zsh]

Options:
  -o, --output <OUTPUT>  Output format: a table for a person; JSON or YAML for a program [default:
                         table] [possible values: table, json, yaml]
      --yes              Confirm a declarative verb without prompting (for automation)
      --color <COLOR>    Color the table's ok/FAIL glyph and verdict [default: auto] [possible
                         values: auto, always, never]
  -h, --help             Print help
"""


class A3FlagValues(unittest.TestCase):
    """A value a flag or argument accepted in the last release is still accepted."""

    def test_the_help_s_possible_values_are_read_even_when_wrapped(self):
        self.assertEqual(compat.parse_values(HELP), {
            "<SHELL>": ["bash", "elvish", "fish", "powershell", "zsh"],
            "--output": ["table", "json", "yaml"],
            "--color": ["auto", "always", "never"],
        })

    def test_the_long_help_form_is_read_too(self):
        long_form = """Options:
  -o, --output <OUTPUT>
          Output format: a table for a person; JSON or YAML for a program
          
          [default: table]
          [possible values: table, json, yaml]

  -C, --repo <DIR>
          The repository root
"""
        self.assertEqual(compat.parse_values(long_form), {"--output": ["table", "json", "yaml"]})

    def surfaces(self):
        base = {"goatbuilder completion": {"flags": {"--output": True}, "args": [{"name": "SHELL", "required": True}],
                                           "values": {"--output": ["table", "json", "yaml"], "<SHELL>": ["bash", "zsh"]}}}
        return base, copy.deepcopy(base)

    def test_the_same_values_pass_and_an_added_value_passes(self):
        base, new = self.surfaces()
        new["goatbuilder completion"]["values"]["--output"].append("csv")
        self.assertEqual(compat.surface_problems(base, new, []), [])

    def test_a_removed_value_is_refused(self):
        base, new = self.surfaces()
        new["goatbuilder completion"]["values"]["--output"].remove("yaml")
        p = compat.surface_problems(base, new, [])
        self.assertEqual(len(p), 1, p)
        self.assertIn("--output", p[0])
        self.assertIn("yaml", p[0])

    def test_a_removed_argument_value_is_refused(self):
        base, new = self.surfaces()
        new["goatbuilder completion"]["values"]["<SHELL>"] = ["bash"]
        self.assertIn("zsh", compat.surface_problems(base, new, [])[0])

    def test_a_flag_that_stops_listing_values_is_refused(self):
        base, new = self.surfaces()
        del new["goatbuilder completion"]["values"]["--output"]
        self.assertTrue(compat.surface_problems(base, new, []))

    def test_a_deprecation_entry_names_the_value_and_it_passes(self):
        base, new = self.surfaces()
        new["goatbuilder completion"]["values"]["--output"].remove("yaml")
        deps = [{"command": "goatbuilder completion", "flag": "--output", "value": "yaml", "replacement": "json", "deprecated_in": "goatbuilder/v0.6.0"}]
        self.assertEqual(compat.surface_problems(base, new, deps), [])

    def test_a_value_entry_without_its_flag_is_refused(self):
        self.assertTrue(compat.deprecation_problems([{"command": "goatbuilder join-node", "value": "door", "replacement": "mesh", "deprecated_in": "goatbuilder/v0.6.0"}]))

    def test_the_kept_surface_holds_the_release_s_values(self):
        s = compat.Baseline(HERE).surface()
        self.assertEqual(s["goatbuilder"]["values"]["--output"], ["table", "json", "yaml"])
        self.assertIn("zsh", s["goatbuilder completion"]["values"]["<SHELL>"])


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


class A4ExitCodes(unittest.TestCase):
    """A kept exit code moves only with its reason (README, "Changing a surface on purpose")."""

    def test_the_release_s_exit_passes(self):
        self.assertEqual(compat.exit_problems({"name": "d", "exit": 2, "release_exit": 2}), [])

    def test_an_edited_exit_with_no_reason_is_refused(self):
        p = compat.exit_problems({"name": "validate-not-yaml", "exit": 1, "release_exit": 2})
        self.assertEqual(len(p), 1, p)
        self.assertIn("validate-not-yaml", p[0])
        self.assertIn("exit_changed", p[0])

    def test_an_edited_exit_with_its_reason_passes(self):
        d = {"name": "d", "exit": 1, "release_exit": 2, "exit_changed": "a config that is not YAML is GB-025"}
        self.assertEqual(compat.exit_problems(d), [])

    def test_an_empty_reason_is_no_reason(self):
        self.assertTrue(compat.exit_problems({"name": "d", "exit": 1, "release_exit": 2, "exit_changed": " "}))

    def test_a_kept_document_without_the_release_s_exit_is_refused(self):
        self.assertTrue(compat.exit_problems({"name": "d", "exit": 1}))

    def test_a_reason_must_be_words(self):
        for reason in (True, [""], ["x"], 1, {"why": "x"}):
            self.assertTrue(compat.exit_problems({"name": "d", "exit": 1, "release_exit": 2, "exit_changed": reason}), reason)

    def test_a_hand_edited_release_exit_is_refused(self):
        b = compat.Baseline(HERE)
        docs = copy.deepcopy(b.documents())
        d = next(x for x in docs if x["name"] == "validate-not-yaml")
        d["exit"], d["release_exit"] = 1, 1
        p = compat.release_values_problems(b.release, docs, b.meta)
        self.assertEqual(len(p), 1, p)
        self.assertIn("release_exit", p[0])

    def test_the_kept_release_values_match_their_digest(self):
        b = compat.Baseline(HERE)
        self.assertEqual(compat.release_values_problems(b.release, b.documents(), b.meta), [])

    def test_editing_exit_alone_does_not_touch_the_digest(self):
        b = compat.Baseline(HERE)
        docs = copy.deepcopy(b.documents())
        d = next(x for x in docs if x["name"] == "validate-not-yaml")
        d["exit"], d["exit_changed"] = 1, "a config that is not YAML is GB-025"
        self.assertEqual(compat.release_values_problems(b.release, docs, b.meta), [])

    def test_every_kept_document_records_the_release_s_exit(self):
        for d in compat.Baseline(HERE).documents():
            self.assertEqual(compat.exit_problems(d), [], d)


class A4EveryVerbIsKept(unittest.TestCase):
    """Every verb of the last release has a kept -o json document, or says why not.

    The kept set is the promise made checkable: a verb with no kept document
    could drop or retype a field unseen. `not_kept:` in documents/manifest.yaml
    names each verb without one and the reason; it only shrinks.
    """

    def verbs(self):
        return sorted(k.split()[1] for k in compat.Baseline(HERE).surface() if len(k.split()) == 2)

    def test_every_verb_has_a_kept_document_or_a_reason(self):
        b = compat.Baseline(HERE)
        kept = {compat.invocation_verb(d["args"], self.verbs()) for d in b.documents()}
        not_kept = b.not_kept()
        missing = [v for v in self.verbs() if v not in kept and v not in not_kept]
        self.assertEqual(missing, [], "verbs with no kept document and no reason in not_kept")
        for v, why in not_kept.items():
            self.assertTrue(str(why).strip(), v)
            self.assertNotIn(v, kept, f"{v} is kept; remove it from not_kept")

    def test_the_verbs_that_change_state_are_kept(self):
        kept = {compat.invocation_verb(d["args"], self.verbs()) for d in compat.Baseline(HERE).documents()}
        for v in ("deploy", "rollback", "plan", "standup", "teardown"):
            self.assertIn(v, kept)

    def test_an_invocation_on_a_scratch_copy_names_only_kept_fixtures(self):
        for d in compat.Baseline(HERE).documents():
            for f in d.get("scratch") or []:
                self.assertTrue(os.path.isdir(os.path.join(HERE, "documents", "fixtures", f)), (d["name"], f))


class RunsAreIsolated(unittest.TestCase):
    """A kept invocation runs with its own HOME and XDG directories, and its
    scratch path is written back as {scratch} in every form it can take."""

    def setUp(self):
        import tempfile
        self.tmp = tempfile.mkdtemp(prefix="compat-iso-")
        # The test's own stand-in for the developer's HOME, so that a run that is
        # not isolated writes here and never into a real one.
        self.home = os.path.join(self.tmp, "home")
        os.makedirs(self.home)
        self.saved = {k: os.environ.get(k) for k in ("HOME", "XDG_STATE_HOME", "XDG_CONFIG_HOME", "XDG_CACHE_HOME", "XDG_DATA_HOME")}
        os.environ["HOME"] = self.home
        for k in list(self.saved)[1:]:
            os.environ.pop(k, None)
        self.bin = os.path.join(self.tmp, "fake-goatbuilder")
        # Writes into $HOME as `tutor` does, and prints a document naming its
        # scratch argument raw, through realpath, and JSON-escaped.
        with open(self.bin, "w") as f:
            f.write("""#!/bin/sh
mkdir -p "$HOME/.goatbuilder/tutor" && echo x > "$HOME/.goatbuilder/tutor/progress.json"
last=""; for a in "$@"; do last="$a"; done
real=$(cd "$last" && pwd -P)
printf '{"schema":"t/v1","home":"%s","xdg":"%s","raw":"%s","real":"%s"}\\n' "$HOME" "$XDG_STATE_HOME" "$last" "$real"
""")
        os.chmod(self.bin, 0o700)

    def tearDown(self):
        import shutil
        for k, v in self.saved.items():
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v
        shutil.rmtree(self.tmp, ignore_errors=True)

    def test_a_run_writes_nothing_into_the_callers_home(self):
        rc, doc, err = compat.run_document(self.bin, REPO, {"name": "t", "format": "json", "args": ["{scratch}/basic"], "scratch": ["basic"]})
        self.assertIsNone(err)
        self.assertNotEqual(doc["home"], self.home)
        self.assertTrue(doc["xdg"] and not doc["xdg"].startswith(self.home), doc)
        self.assertEqual(os.listdir(self.home), [], "the run wrote into the caller's HOME")

    def test_every_form_of_the_scratch_path_is_written_back(self):
        rc, doc, err = compat.run_document(self.bin, REPO, {"name": "t", "format": "json", "args": ["{scratch}/basic"], "scratch": ["basic"]})
        self.assertIsNone(err)
        self.assertEqual(doc["raw"], "{scratch}/basic")
        self.assertEqual(doc["real"], "{scratch}/basic")
        self.assertNotIn(self.tmp, json.dumps(doc))

    def test_a_json_escaped_scratch_path_is_written_back(self):
        out = compat.unscratch('{"p": "C:\\\\x\\\\y/basic"}', "C:\\x\\y")
        self.assertEqual(json.loads(out)["p"], "{scratch}/basic")


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
        pins = compat.load_pins(REPO)
        self.assertEqual(pins[(b.release, b.meta["binary"])], b.meta["binary_sha256"])

    def test_refresh_never_writes_the_pin_list(self):
        before = open(os.path.join(REPO, compat.PINS), "rb").read()
        compat.refresh(REPO, self.root, "goatbuilder/v0.5.1", self.bin, sums=self.sums("f" * 64))
        self.assertEqual(open(os.path.join(REPO, compat.PINS), "rb").read(), before)


class OnePinList(unittest.TestCase):
    """compat and the upgrade tests (gate E1 to E3) read one list of published checksums.

    crates/goatbuilder/tests/upgrade/releases.sha256 is that list. fetch.sh reads
    it with `read -r want path` (any whitespace), released.rs with
    `split_once("  ")` (exactly two spaces), compat with load_pins: all three
    must read the same thing, so every line has exactly the shape SHA256SUMS writes.
    """

    LINE = re.compile(r"^[0-9a-f]{64}  v\d+\.\d+\.\d+/goatbuilder-(linux|darwin)-(amd64|arm64)$")

    def lines(self):
        with open(os.path.join(REPO, compat.PINS)) as f:
            return [l.rstrip("\n") for l in f if l.strip() and not l.startswith("#")]

    def test_the_list_is_the_upgrade_tests_and_compat_keeps_none_of_its_own(self):
        self.assertEqual(compat.PINS, "crates/goatbuilder/tests/upgrade/releases.sha256")
        self.assertFalse(os.path.exists(os.path.join(HERE, "releases.sha256")))

    def test_every_line_has_the_shape_both_readers_accept(self):
        lines = self.lines()
        self.assertTrue(lines)
        for l in lines:
            self.assertRegex(l, self.LINE)
        paths = [l.split("  ", 1)[1] for l in lines]
        self.assertEqual(len(paths), len(set(paths)), "a path is pinned twice")

    def test_fetch_sh_released_rs_and_compat_read_the_same_pins(self):
        lines = self.lines()
        as_fetch = {}
        as_released = {}
        for l in lines:
            want, path = l.split(None, 1)  # read -r want path
            as_fetch[path.strip()] = want
            sha, p = l.split("  ", 1)  # split_once("  ")
            as_released[p] = sha
        as_compat = {f"{tag.split('/', 1)[1]}/{asset}": sha for (tag, asset), sha in compat.load_pins(REPO).items()}
        self.assertEqual(as_fetch, as_released)
        self.assertEqual(as_compat, as_released)


@unittest.skipUnless(os.environ.get("GOATBUILDER_BIN"), "set GOATBUILDER_BIN to a built goatbuilder")
class TheBuildKeepsThePromise(unittest.TestCase):
    def test_every_surface_against_the_last_release(self):
        rows = compat.check_all(REPO, HERE, os.environ["GOATBUILDER_BIN"])
        failed = [r for r in rows if not r[0]]
        self.assertEqual(failed, [], json.dumps(failed, indent=1))

    def test_a_whole_check_leaves_the_callers_home_as_it_was(self):
        import shutil
        import tempfile
        home = tempfile.mkdtemp(prefix="compat-caller-home-")
        saved = os.environ.get("HOME")
        os.environ["HOME"] = home
        try:
            compat.check_all(REPO, HERE, os.environ["GOATBUILDER_BIN"])
            self.assertEqual(os.listdir(home), [], "a check wrote into the caller's HOME")
        finally:
            os.environ["HOME"] = saved
            shutil.rmtree(home, ignore_errors=True)


if __name__ == "__main__":
    unittest.main(verbosity=2)
