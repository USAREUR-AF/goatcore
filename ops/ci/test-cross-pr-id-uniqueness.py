#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Mocked tests for check-cross-pr-id-uniqueness.py; no network access."""

from __future__ import annotations

import contextlib
import importlib.util
import io
import json
import pathlib
import unittest
import urllib.error


SUBJECT = pathlib.Path(__file__).with_name("check-cross-pr-id-uniqueness.py")
SPEC = importlib.util.spec_from_file_location("cross_pr_id_guard", SUBJECT)
assert SPEC and SPEC.loader
guard = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(guard)


def changed(path, change_type="ADDED"):
    return {"path": path, "changeType": change_type}


def pull(number, files, *, changed_files=None, truncated=False):
    return {
        "number": number,
        "url": f"https://github.com/usareur-af/goatcore/pull/{number}",
        "changedFiles": len(files) if changed_files is None else changed_files,
        "files": {
            "pageInfo": {"hasNextPage": truncated},
            "nodes": files,
        },
    }


def payload(*prs, truncated=False, total=None, current_pr=20):
    return {
        "data": {
            "repository": {
                "current": next((pr for pr in prs if pr["number"] == current_pr), None),
                "pullRequests": {
                    "totalCount": len(prs) if total is None else total,
                    "pageInfo": {"hasNextPage": truncated},
                    "nodes": list(prs),
                }
            }
        }
    }


class Response:
    def __init__(self, value):
        self.value = value

    def __enter__(self):
        return self

    def __exit__(self, *_args):
        return False

    def read(self):
        return json.dumps(self.value).encode()


class Opener:
    def __init__(self, value=None, error=None):
        self.value = value
        self.error = error
        self.calls = 0
        self.request = None

    def __call__(self, request, timeout):
        self.calls += 1
        self.request = request
        self.timeout = timeout
        if self.error:
            raise self.error
        return Response(self.value)


class CrossPrIdTests(unittest.TestCase):
    ENV = {
        "GITHUB_TOKEN": "not-a-real-token",
        "GITHUB_REPOSITORY": "USAREUR-AF/goatcore",
        "PR_NUMBER": "20",
    }

    def execute(self, value=None, error=None):
        opener = Opener(value=value, error=error)
        stdout, stderr = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            result = guard.run(dict(self.ENV), opener=opener)
        return result, stdout.getvalue(), stderr.getvalue(), opener

    def test_unique_claims_pass_with_exactly_one_api_call(self):
        result, out, err, opener = self.execute(
            payload(
                pull(20, [changed("docs/findings/F-600-current.md")]),
                pull(19, [changed("docs/adr/1200-other.md")]),
            )
        )
        self.assertEqual(result, 0)
        self.assertIn("OK", out)
        self.assertEqual(err, "")
        self.assertEqual(opener.calls, 1)
        self.assertEqual(opener.request.get_method(), "POST")

    def test_confirmed_finding_collision_fails_and_names_competing_pr(self):
        result, _out, err, opener = self.execute(
            payload(
                pull(20, [changed("docs/findings/F-600-current.md")]),
                pull(17, [changed("docs/findings/F-600-other.md")]),
            )
        )
        self.assertEqual(result, 1)
        self.assertIn("FND F-600", err)
        self.assertIn("PR #17", err)
        self.assertIn("https://github.com/usareur-af/goatcore/pull/17", err)
        self.assertEqual(opener.calls, 1)

    def test_confirmed_adr_collision_fails(self):
        result, _out, err, _opener = self.execute(
            payload(
                pull(20, [changed("docs/adr/1200-current.md", "RENAMED")]),
                pull(18, [changed("docs/adr/1200-other.md")]),
            )
        )
        self.assertEqual(result, 1)
        self.assertIn("ADR 1200", err)

    def test_modified_deleted_and_non_markdown_paths_do_not_claim_ids(self):
        current = pull(
            20,
            [
                changed("docs/findings/F-600-current.md", "MODIFIED"),
                changed("docs/adr/1200-current.md", "DELETED"),
                changed("docs/findings/F-601-report.html"),
                changed("docs/findings/INDEX.md"),
            ],
        )
        other = pull(
            19,
            [
                changed("docs/findings/F-600-other.md"),
                changed("docs/adr/1200-other.md"),
                changed("docs/findings/F-601-other.md"),
            ],
        )
        result, out, _err, _opener = self.execute(payload(current, other))
        self.assertEqual(result, 0)
        self.assertIn("OK", out)

    def test_same_pr_is_not_a_cross_pr_collision(self):
        result, out, _err, _opener = self.execute(
            payload(
                pull(
                    20,
                    [
                        changed("docs/findings/F-600-one.md"),
                        changed("docs/findings/F-600-two.md"),
                    ],
                )
            )
        )
        self.assertEqual(result, 0)
        self.assertIn("OK", out)

    def test_collision_still_fails_when_another_pr_is_truncated(self):
        result, out, err, _opener = self.execute(
            payload(
                pull(20, [changed("docs/findings/F-600-current.md")]),
                pull(
                    19,
                    [changed("docs/findings/F-600-visible.md")],
                    changed_files=101,
                    truncated=True,
                ),
            )
        )
        self.assertEqual(result, 1)
        self.assertIn("only its first 100", out)
        self.assertIn("PR #19", err)

    def test_incomplete_results_warn_and_pass_without_confirmed_collision(self):
        result, out, err, _opener = self.execute(
            payload(
                pull(20, [changed("docs/findings/F-600-current.md")]),
                pull(19, [], changed_files=101, truncated=True),
                truncated=True,
                total=101,
            )
        )
        self.assertEqual(result, 0)
        self.assertIn("only the first 100 of 101 open PRs", out)
        self.assertIn("PR #19 has 101 files", out)
        self.assertIn("WARNING", out)
        self.assertEqual(err, "")

    def test_missing_current_pr_warns_and_passes(self):
        result, out, _err, _opener = self.execute(
            payload(pull(19, [changed("docs/findings/F-600-other.md")]))
        )
        self.assertEqual(result, 0)
        self.assertIn("current PR #20", out)

    def test_graphql_errors_warn_and_pass(self):
        result, out, _err, opener = self.execute({"errors": [{"message": "rate limited"}]})
        self.assertEqual(result, 0)
        self.assertIn("could not be verified", out)
        self.assertEqual(opener.calls, 1)

    def test_malformed_success_response_warns_and_passes(self):
        result, out, _err, opener = self.execute(
            {"data": {"repository": {"current": {}, "pullRequests": None}}}
        )
        self.assertEqual(result, 0)
        self.assertIn("could not be verified", out)
        self.assertEqual(opener.calls, 1)

    def test_transport_errors_warn_and_pass(self):
        result, out, _err, opener = self.execute(
            error=urllib.error.URLError("offline")
        )
        self.assertEqual(result, 0)
        self.assertIn("could not be verified", out)
        self.assertNotIn("offline", out)
        self.assertEqual(opener.calls, 1)

    def test_missing_environment_warns_without_an_api_call(self):
        opener = Opener(payload())
        stdout = io.StringIO()
        with contextlib.redirect_stdout(stdout):
            result = guard.run({}, opener=opener)
        self.assertEqual(result, 0)
        self.assertIn("unavailable", stdout.getvalue())
        self.assertEqual(opener.calls, 0)


if __name__ == "__main__":
    unittest.main()
