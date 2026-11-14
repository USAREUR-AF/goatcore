"""The audit bucket's stalled multipart uploads are cleaned up, or the deploy fails.

roles/audit-subscriber/tasks/lifecycle.yml sends an AbortIncompleteMultipartUpload
clause in the bucket's lifecycle policy. Some MinIO releases accept the import,
exit 0 and store the policy without that clause. The task used to print a warning
and carry on, so a bucket could run with no cleanup of stalled uploads behind a
green deploy.

MinIO also cleans up stalled uploads itself, server-wide, after its `api`
setting `stale_uploads_expiry` (MINIO_API_STALE_UPLOADS_EXPIRY). The task now
reads that setting back, and the deploy passes only when the clause is stored
or the server's expiry is no longer than the clause asks for.

The test renders the task's own Jinja with the filters it uses, so it holds the
task as written, not a copy of its logic.

Run: python3 -m pytest -q tests/test_audit_lifecycle_multipart.py
"""
import json
import os
import re
import unittest

import jinja2
import yaml

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TASKS = os.path.join(REPO, "ops/ansible/roles/audit-subscriber/tasks/lifecycle.yml")

DECIDE = "audit-subscriber | lifecycle | decide whether stalled multipart uploads are cleaned up"
CHECK = "audit-subscriber | lifecycle | assert stalled multipart uploads are cleaned up"


def walk(tasks):
    for t in tasks or []:
        yield t
        for key in ("block", "rescue", "always"):
            yield from walk(t.get(key))


def task(name):
    with open(TASKS, encoding="utf-8") as fh:
        doc = yaml.safe_load(fh)
    for t in walk(doc):
        if t.get("name") == name:
            return t
    raise AssertionError(f"no task named {name!r} in {TASKS}")


def env():
    # Ansible renders these as plain text, never HTML: nothing is escaped.
    e = jinja2.Environment(undefined=jinja2.ChainableUndefined, autoescape=jinja2.select_autoescape(default_for_string=False))
    e.filters["from_json"] = json.loads
    e.filters["regex_findall"] = lambda s, pat: re.findall(pat, s)
    e.filters["bool"] = lambda v: v if isinstance(v, bool) else str(v).strip().lower() in ("true", "yes", "1", "on")
    return e


def stored_policy(with_abort):
    rule = {
        "ID": "audit-integrity-three-tier",
        "Status": "Enabled",
        "Expiration": {"Days": 365},
        "NoncurrentVersionExpiration": {"NoncurrentDays": 1},
    }
    if with_abort:
        rule["AbortIncompleteMultipartUpload"] = {"DaysAfterInitiation": 1}
    return json.dumps({"Rules": [rule]})


def api_config(expiry):
    return (
        "api requests_max=0 requests_deadline=10s cluster_deadline=10s cors_allow_origin=* "
        "remote_transport_deadline=2h list_quorum=strict stale_uploads_cleanup_interval=6h "
        f"stale_uploads_expiry={expiry} delete_cleanup_interval=5m"
    )


def passes(policy, config_rc, config_out, days=1):
    e = env()
    ctx = {
        "audit_subscriber_lifecycle_applied": {"stdout": policy},
        "audit_subscriber_lifecycle_api_config": {"rc": config_rc, "stdout": config_out},
        "audit_subscriber_minio_multipart_abort_days": days,
    }
    for key, tmpl in task(DECIDE)["ansible.builtin.set_fact"].items():
        ctx[key] = e.from_string(tmpl).render(**ctx).strip()
    return all(bool(e.compile_expression(c)(**ctx)) for c in task(CHECK)["ansible.builtin.assert"]["that"])


class StalledUploads(unittest.TestCase):
    def test_the_clause_stored_passes_whatever_the_server_says(self):
        self.assertTrue(passes(stored_policy(True), 0, api_config("72h")))
        self.assertTrue(passes(stored_policy(True), 1, ""))

    def test_the_clause_dropped_passes_on_the_servers_own_cleanup(self):
        self.assertTrue(passes(stored_policy(False), 0, api_config("24h")))
        self.assertTrue(passes(stored_policy(False), 0, api_config("1h30m0s")))

    def test_the_clause_dropped_and_a_longer_server_expiry_fails(self):
        self.assertFalse(passes(stored_policy(False), 0, api_config("48h")))
        self.assertFalse(passes(stored_policy(False), 0, api_config("24h0m1s")))

    def test_the_clause_dropped_and_the_server_setting_unreadable_fails(self):
        self.assertFalse(passes(stored_policy(False), 1, ""))
        self.assertFalse(passes(stored_policy(False), 0, "api requests_max=0"))

    def test_a_longer_clause_allows_a_longer_server_expiry(self):
        self.assertTrue(passes(stored_policy(False), 0, api_config("48h"), days=2))

    def test_the_drop_is_no_longer_only_a_warning(self):
        with open(TASKS, encoding="utf-8") as fh:
            text = fh.read()
        self.assertFalse("Not fatal" in text, "the task still treats a dropped clause as not fatal")


if __name__ == "__main__":
    unittest.main()
