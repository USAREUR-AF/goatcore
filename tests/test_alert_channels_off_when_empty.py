#!/usr/bin/env python3
"""Outside alert channels are optional: empty turns each off, and no role requires one.

docs/design/pluggable-dependencies.md row 16. The channels are PagerDuty, Slack and
alert e-mail (PAGERDUTY_INTEGRATION_KEY, SLACK_ALERT_WEBHOOK_URL, ALERT_SMTP_*,
ALERT_EMAIL_DIGEST_RECIPIENTS); ops/secrets/off-when-empty.txt names them.

1. every group_vars binding of those names falls back to '' (empty is a declared value),
   and every name it binds is listed in off-when-empty.txt
2. no task, play or role default REQUIRES one: no `mandatory`, no assert or fail that
   reads the variable, no role-default lookup without an empty fallback
3. both Alertmanager templates (roles/obs-host, roles/observability) rendered by
   Ansible with every channel empty carry no PagerDuty, Slack or e-mail receiver and
   no SMTP relay; with them set (control) they carry all three
4. with Docker (REQUIRE_DOCKER=1 in CI): `amtool check-config` in the role's pinned
   Alertmanager image accepts the empty-channel render
5. the deploy presence check (deploy-secrets-needed.py) never reports a channel as
   MISSING; the goatnet's own ntfy channel still is reported (control)

Run: python3 tests/test_alert_channels_off_when_empty.py
"""
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import unittest

import yaml

REPO = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
ANSIBLE = os.path.join(REPO, "ops/ansible")
OFF_FILE = os.path.join(REPO, "ops/secrets/off-when-empty.txt")
CHANNEL_ENV = re.compile(r"^(PAGERDUTY_INTEGRATION_KEY(_EU)?|SLACK_ALERT_WEBHOOK_URL|ALERT_SMTP_[A-Z_]+|ALERT_EMAIL_DIGEST_RECIPIENTS)$")
CHANNEL_VARS = ["pagerduty_integration_key", "slack_webhook_url", "smtp_smarthost", "smtp_auth_username",
                "smtp_auth_password", "email_digest_recipients", "deadman_watch_routing_key"]
TEMPLATES = ["roles/obs-host/templates/alertmanager.yml.j2", "roles/observability/templates/alertmanager.yml.j2"]
LOOKUP = re.compile(r"""lookup\(\s*['"]env['"]\s*,\s*['"]([A-Z][A-Z0-9_]*)['"]\s*\)""")


def off_names():
    out = set()
    with open(OFF_FILE, encoding="utf-8") as fh:
        for line in fh:
            w = line.split("#", 1)[0].split()
            if w:
                out.add(w[0])
    return out


def yaml_files(*roots):
    for root in roots:
        for d, _sub, files in os.walk(root):
            for f in files:
                if f.endswith((".yml", ".yaml")):
                    yield os.path.join(d, f)


class Declarations(unittest.TestCase):
    def test_every_channel_binding_falls_back_to_empty_and_is_listed(self):
        off = off_names()
        seen = set()
        for path in yaml_files(os.path.join(ANSIBLE, "group_vars"), os.path.join(ANSIBLE, "roles")):
            with open(path, encoding="utf-8", errors="replace") as fh:
                for n, line in enumerate(fh, 1):
                    for name in LOOKUP.findall(line):
                        if not CHANNEL_ENV.match(name):
                            continue
                        seen.add(name)
                        self.assertRegex(line, r"default\(\s*''\s*,\s*true\s*\)",
                                         "%s:%d binds %s with no empty fallback" % (os.path.relpath(path, REPO), n, name))
                        self.assertIn(name, off, "%s is bound in %s but not listed in ops/secrets/off-when-empty.txt"
                                      % (name, os.path.relpath(path, REPO)))
        if not os.path.isdir(os.path.join(ANSIBLE, "group_vars", "zagros_dev_hosts")):
            self.skipTest("no goatnet in this tree binds the alert channels (a goatcore checkout carries none)")
        self.assertTrue({"PAGERDUTY_INTEGRATION_KEY", "SLACK_ALERT_WEBHOOK_URL", "ALERT_SMTP_SMARTHOST"} <= seen,
                        "the scan found no channel bindings: it is not reading the tree")

    def test_no_task_play_or_default_requires_a_channel(self):
        roots = [os.path.join(ANSIBLE, "roles")]
        plays = [os.path.join(ANSIBLE, f) for f in os.listdir(ANSIBLE) if f.endswith(".yml")]
        bad = []
        for path in list(yaml_files(*roots)) + plays:
            rel = os.path.relpath(path, REPO)
            with open(path, encoding="utf-8", errors="replace") as fh:
                text = fh.read()
            for var in CHANNEL_VARS:
                for line in text.splitlines():
                    if var in line and "mandatory" in line:
                        bad.append("%s: %s | mandatory" % (rel, var))
            try:
                docs = list(yaml.safe_load_all(text))
            except yaml.YAMLError:
                continue
            for task in walk_tasks(docs):
                for key in ("assert", "ansible.builtin.assert"):
                    if isinstance(task.get(key), dict):
                        that = json.dumps(task[key].get("that"))
                        bad += ["%s: assert reads %s" % (rel, v) for v in CHANNEL_VARS if v in that]
                for key in ("fail", "ansible.builtin.fail"):
                    if key in task:
                        when = json.dumps(task.get("when"))
                        bad += ["%s: fail when %s" % (rel, v) for v in CHANNEL_VARS if v in when]
        self.assertEqual(bad, [], "a role requires an outside alert channel:\n  " + "\n  ".join(bad))

    def test_negative_control_the_scan_sees_a_requiring_role(self):
        task = {"name": "x", "ansible.builtin.assert": {"that": ["pagerduty_integration_key | length > 0"]}}
        found = [t for t in walk_tasks([[{"block": [task]}]]) if "ansible.builtin.assert" in t]
        self.assertEqual(len(found), 1, "walk_tasks must reach an assert nested in a block")


def walk_tasks(node):
    if isinstance(node, list):
        for x in node:
            yield from walk_tasks(x)
    elif isinstance(node, dict):
        yield node
        for k in ("block", "rescue", "always", "tasks", "pre_tasks", "post_tasks", "handlers"):
            if k in node:
                yield from walk_tasks(node[k])


EMPTY = {"pagerduty_integration_key": "", "slack_webhook_url": "", "smtp_smarthost": "", "smtp_auth_username": "",
         "smtp_auth_password": "", "email_digest_recipients": "", "deadman_webhook_url": "",
         "ntfy_alertmanager_webhook_url": "http://alertmanager-ntfy:8000/hook", "site_id": "pasture"}
SET = dict(EMPTY, pagerduty_integration_key="pd-fixture-key", slack_webhook_url="https://hooks.example.test/x",
           smtp_smarthost="smtp.example.test:587", smtp_auth_username="u", smtp_auth_password="p",
           email_digest_recipients="ops@example.test")


def render(template, values, out_dir):
    """The template rendered by Ansible's own template module, as the role renders it."""
    play = os.path.join(out_dir, "render.yml")
    with open(play, "w") as fh:
        fh.write("- hosts: localhost\n  gather_facts: false\n  tasks:\n"
                 "    - ansible.builtin.template:\n        src: \"{{ src }}\"\n        dest: \"{{ dest }}\"\n")
    varfile = os.path.join(out_dir, "vars.json")
    dest = os.path.join(out_dir, "alertmanager.yml")
    with open(varfile, "w") as fh:
        json.dump(dict(values, src=os.path.join(ANSIBLE, template), dest=dest), fh)
    r = subprocess.run(["ansible-playbook", "-i", "localhost,", "-c", "local", play, "-e", "@" + varfile],
                       capture_output=True, text=True, env=dict(os.environ, ANSIBLE_NOCOLOR="1"))
    if r.returncode != 0:
        raise AssertionError("render of %s failed:\n%s%s" % (template, r.stdout[-3000:], r.stderr[-2000:]))
    with open(dest) as fh:
        return fh.read()


def receivers(doc):
    return {r["name"]: r for r in doc.get("receivers") or []}


@unittest.skipUnless(shutil.which("ansible-playbook"), "ansible-playbook not installed")
class Rendered(unittest.TestCase):
    def setUp(self):
        self.d = tempfile.mkdtemp()

    def tearDown(self):
        shutil.rmtree(self.d, ignore_errors=True)

    def test_empty_channels_render_no_outside_receiver(self):
        for t in TEMPLATES:
            text = render(t, EMPTY, self.d)
            doc = yaml.safe_load(text)
            self.assertNotIn("smtp_smarthost", doc.get("global") or {}, t)
            for name, r in receivers(doc).items():
                for key in ("pagerduty_configs", "slack_configs", "email_configs"):
                    self.assertNotIn(key, r, "%s: receiver %s carries %s with every channel empty" % (t, name, key))
            self.assertIn("webhook-noop", receivers(doc), t)
            self.assertIn("alertmanager-ntfy", text, "%s: the goatnet's own ntfy channel is not rendered" % t)
            if os.environ.get("REQUIRE_DOCKER") == "1" or docker_ok():
                amtool_check(t, text, self.d)

    def test_control_set_channels_render_every_outside_receiver(self):
        for t in TEMPLATES:
            doc = yaml.safe_load(render(t, SET, self.d))
            keys = {k for r in receivers(doc).values() for k in r}
            self.assertTrue({"pagerduty_configs", "slack_configs", "email_configs"} <= keys,
                            "%s: with every channel set the render lacks one: %s" % (t, sorted(keys)))
            self.assertEqual((doc.get("global") or {}).get("smtp_smarthost"), "smtp.example.test:587", t)


def docker_ok():
    return shutil.which("docker") is not None and subprocess.run(["docker", "info"], capture_output=True).returncode == 0


def amtool_check(template, text, d):
    """Alertmanager's own config check, in the image the obs-host role pins."""
    if not docker_ok():
        raise AssertionError("REQUIRE_DOCKER=1 and no Docker daemon is reachable")
    with open(os.path.join(ANSIBLE, "roles/obs-host/defaults/main.yml")) as fh:
        image = yaml.safe_load(fh)["alertmanager_image"]
    cfg = os.path.join(d, "check.yml")
    with open(cfg, "w") as fh:
        fh.write(text)
    r = subprocess.run(["docker", "run", "--rm", "--entrypoint", "amtool", "-v", "%s:/c/alertmanager.yml:ro" % cfg,
                        image, "check-config", "/c/alertmanager.yml"], capture_output=True, text=True)
    if r.returncode != 0:
        raise AssertionError("amtool check-config refused the empty-channel render of %s:\n%s%s"
                             % (template, r.stdout, r.stderr))
    print("amtool check-config ok: %s (%s)" % (template, image), file=sys.stderr)


class PresenceCheck(unittest.TestCase):
    def test_deploy_presence_check_never_reports_a_channel_missing(self):
        env = {k: v for k, v in os.environ.items() if not CHANNEL_ENV.match(k) and k != "NTFY_ALERTMANAGER_WEBHOOK_URL"}
        env["GOAT_SECRET_STORE"] = "env"
        groups = [g for g in ("zagros_dev_hosts", "zagros_hosts", "zagros_prod_obs_hosts", "integdev_hosts")
                  if os.path.isdir(os.path.join(ANSIBLE, "group_vars", g))]
        if not groups:
            self.skipTest("no goatnet with an observability host in this tree (a goatcore checkout carries none)")
        for group in groups:
            r = subprocess.run([sys.executable, os.path.join(REPO, "ops/scripts/deploy-secrets-needed.py"), "--repo", REPO,
                                "--component", "observability", "--ledger", "obs-host",
                                "--play", os.path.join(ANSIBLE, "obs.yml"), group],
                               capture_output=True, text=True, env=env)
            self.assertIn(r.returncode, (0, 1), r.stderr)
            reported = {line.split()[1] for line in r.stdout.splitlines() if len(line.split()) >= 3}
            leaked = sorted(n for n in reported for a in n.split("|") if CHANNEL_ENV.match(a))
            self.assertEqual(leaked, [], "%s: the presence check requires an outside channel" % group)
            if group == "zagros_dev_hosts":
                self.assertIn("NTFY_ALERTMANAGER_WEBHOOK_URL", reported,
                              "control: the goatnet's own ntfy channel must still be a need")


if __name__ == "__main__":
    unittest.main(verbosity=2)
