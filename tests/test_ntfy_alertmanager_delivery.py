#!/usr/bin/env python3
"""Alertmanager to ntfy delivery, end to end, on the pinned images.

An alert storm that shares one alertname lands in one Alertmanager group, so
one webhook POST carries all of it. ntfy renders the notification server-side
and refuses a render over its 4096-byte message limit with HTTP 400, which
Alertmanager drops without retry. A render test cannot see that; only a real
Alertmanager posting to a real ntfy can.

Both configs come from the repo exactly as the roles render them: the obs-host
alertmanager.yml.j2 and the relay-health ntfy-server.yml.j2 (deny-all auth,
write-only publisher), with the relay-health template-dir mounted. The only
harness change is shorter group timers so the test does not wait 30 seconds.

  bounded  the shipped shape. 40 criticals in one group reach the topic as
           exactly one notification: 15 alerts sent (max_alerts), five listed,
           "+ 10 more firing" and "+ 25 more not included by Alertmanager".
           Alertmanager counts no failed notification.
  control  the previous shape: max_alerts 0 and ntfy's built-in alertmanager
           template. The same storm is refused, Alertmanager counts the failure
           and nothing reaches the topic. This proves the harness sees the
           failure it guards against.

Needs Docker and ansible-playbook. Skips without them unless REQUIRE_DOCKER=1,
which is how CI runs it.
"""
import base64
import json
import os
import re
import secrets
import shutil
import subprocess
import sys
import tempfile
import time
import unittest
import urllib.error
import urllib.request

import yaml

REPO = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
ANSIBLE = os.path.join(REPO, "ops/ansible")
AM_TEMPLATE = "roles/obs-host/templates/alertmanager.yml.j2"
NTFY_TEMPLATE = "roles/relay-health/templates/ntfy-server.yml.j2"
NTFY_TEMPLATE_DIR = os.path.join(ANSIBLE, "roles/relay-health/files/ntfy-templates")
TOPIC = "goat-pasture-critical"
STORM = 40


def ntfy_image():
    with open(os.path.join(REPO, "ops/compose/relay-health-standalone/docker-compose.public.yml")) as fh:
        text = fh.read()
    return re.search(r"\$\{NTFY_IMAGE:-([^}]+)\}", text).group(1)


def alertmanager_image():
    with open(os.path.join(ANSIBLE, "roles/obs-host/defaults/main.yml")) as fh:
        defaults = yaml.safe_load(fh)
    return defaults["alertmanager_image"]


def docker_ok():
    return shutil.which("docker") is not None and subprocess.run(
        ["docker", "info"], capture_output=True).returncode == 0


def render(template, values, out_dir, name):
    """A template rendered by Ansible's own template module, as the role renders it."""
    play = os.path.join(out_dir, "render-%s.yml" % name)
    with open(play, "w") as fh:
        fh.write("- hosts: localhost\n  gather_facts: false\n  tasks:\n"
                 "    - ansible.builtin.template:\n        src: \"{{ src }}\"\n        dest: \"{{ dest }}\"\n")
    varfile = os.path.join(out_dir, "vars-%s.json" % name)
    dest = os.path.join(out_dir, name)
    with open(varfile, "w") as fh:
        json.dump(dict(values, src=os.path.join(ANSIBLE, template), dest=dest), fh)
    r = subprocess.run(["ansible-playbook", "-i", "localhost,", "-c", "local", play, "-e", "@" + varfile],
                       capture_output=True, text=True, env=dict(os.environ, ANSIBLE_NOCOLOR="1"))
    if r.returncode != 0:
        raise AssertionError("render of %s failed:\n%s%s" % (template, r.stdout[-3000:], r.stderr[-2000:]))
    return dest


def quicken(route):
    """Shorter group timers, so the test does not wait out the production ones."""
    for key in ("group_wait", "group_interval"):
        if key in route:
            route[key] = "1s" if key == "group_wait" else "5s"
    for child in route.get("routes") or []:
        quicken(child)


def ntfy_webhooks(doc):
    return [cfg for r in doc["receivers"] if r["name"] in ("critical-fanout", "warning-fanout")
            for cfg in r.get("webhook_configs", [])]


def storm():
    # Real alerts carry a long description and a generator URL each; that is
    # what pushed the built-in template over the limit in production.
    description = ("The probe could not complete a TLS handshake with this endpoint. Check that the service is "
                   "running, that its certificate chain is complete, and that the probe trusts the issuer. "
                   "The alert clears on the next successful probe. ") * 2
    return [{"labels": {"alertname": "DeliveryStorm", "severity": "critical", "site": "pasture",
                        "component": "fixture", "instance": "host-%02d.pasture.example.test:443" % i},
             "annotations": {"summary": "TLS handshake failed on host-%02d.pasture.example.test" % i,
                             "description": description},
             "generatorURL": "http://prometheus.pasture.example.test:9090/graph?g0.expr=probe_success%%7Binstance%%3D"
                             "%%22host-%02d%%22%%7D%%20%%3D%%3D%%200&g0.tab=1" % i}
            for i in range(STORM)]


class Harness:
    def __init__(self, case, workdir):
        self.case, self.d = case, workdir
        self.tag = "goat-ntfy-%s-%s" % (case, secrets.token_hex(3))
        self.net = self.tag
        self.admin_pw = secrets.token_urlsafe(24)
        self.pub_pw = secrets.token_urlsafe(24)
        self.containers = []

    def sh(self, *argv, env=None, check=True):
        r = subprocess.run(list(argv), capture_output=True, text=True, env=env)
        if check and r.returncode != 0:
            raise AssertionError("%s failed: %s%s" % (" ".join(argv[:4]), r.stdout[-1500:], r.stderr[-1500:]))
        return r.stdout.strip()

    def port(self, name, inner):
        return self.sh("docker", "port", name, str(inner)).splitlines()[0].rsplit(":", 1)[1]

    def wait(self, url, auth=None, timeout=60):
        deadline = time.time() + timeout
        while time.time() < deadline:
            try:
                req = urllib.request.Request(url)
                if auth:
                    req.add_header("Authorization", auth)
                urllib.request.urlopen(req, timeout=3).read()
                return
            except (urllib.error.URLError, ConnectionError, OSError):
                time.sleep(1)
        raise AssertionError("%s never answered\n%s" % (url, self.logs()))

    def up(self, mutate_alertmanager):
        server = render(NTFY_TEMPLATE, {"ntfy_fqdn": "ntfy.pasture.example.test", "ntfy_cache_duration": "1h"},
                        self.d, "server.yml")
        self.sh("docker", "network", "create", self.net)
        ntfy = self.tag + "-ntfy"
        self.containers.append(ntfy)
        self.sh("docker", "run", "-d", "--name", ntfy, "--network", self.net, "--network-alias", "ntfy",
                "-p", "127.0.0.1::80",
                # The role mounts the cache and data dirs from the host.
                "--tmpfs", "/var/cache/ntfy", "--tmpfs", "/var/lib/ntfy",
                "-v", server + ":/etc/ntfy/server.yml:ro",
                "-v", NTFY_TEMPLATE_DIR + ":/etc/ntfy/templates:ro",
                ntfy_image(), "serve")
        self.ntfy_url = "http://127.0.0.1:%s" % self.port(ntfy, 80)
        self.wait(self.ntfy_url + "/v1/health")
        # The relay-health role's users and grants: admin reads, alertmanager is write-only.
        for user, role, pw in (("admin", "admin", self.admin_pw), ("alertmanager", "user", self.pub_pw)):
            self.sh("docker", "exec", "-e", "NTFY_PASSWORD=" + pw, ntfy, "ntfy", "user", "add",
                    "--role=" + role, user)
        self.sh("docker", "exec", ntfy, "ntfy", "access", "alertmanager", TOPIC, "write-only")

        values = {"pagerduty_integration_key": "", "slack_webhook_url": "", "smtp_smarthost": "",
                  "smtp_auth_username": "", "smtp_auth_password": "", "email_digest_recipients": "",
                  "deadman_webhook_url": "", "site_id": "pasture",
                  "ntfy_alertmanager_webhook_url": "http://ntfy/%s?template=goat-alertmanager" % TOPIC,
                  "ntfy_alertmanager_user": "alertmanager", "ntfy_alertmanager_password": self.pub_pw}
        path = render(AM_TEMPLATE, values, self.d, "alertmanager.yml")
        with open(path) as fh:
            doc = yaml.safe_load(fh)
        quicken(doc["route"])
        mutate_alertmanager(doc)
        with open(path, "w") as fh:
            yaml.safe_dump(doc, fh)
        os.chmod(path, 0o644)
        am = self.tag + "-am"
        self.containers.append(am)
        self.sh("docker", "run", "-d", "--name", am, "--network", self.net, "-p", "127.0.0.1::9093",
                "-v", path + ":/etc/alertmanager/alertmanager.yml:ro", alertmanager_image(),
                "--config.file=/etc/alertmanager/alertmanager.yml")
        self.am_url = "http://127.0.0.1:%s" % self.port(am, 9093)
        self.wait(self.am_url + "/-/ready")

    def fire(self):
        body = json.dumps(storm()).encode()
        req = urllib.request.Request(self.am_url + "/api/v2/alerts", data=body, method="POST",
                                     headers={"Content-Type": "application/json"})
        urllib.request.urlopen(req, timeout=10).read()

    def messages(self):
        auth = "Basic " + base64.b64encode(("admin:" + self.admin_pw).encode()).decode()
        req = urllib.request.Request(self.ntfy_url + "/%s/json?poll=1&since=all" % TOPIC,
                                     headers={"Authorization": auth})
        lines = urllib.request.urlopen(req, timeout=10).read().decode().splitlines()
        return [m for m in (json.loads(line) for line in lines if line.strip()) if m.get("event") == "message"]

    def counters(self):
        text = urllib.request.urlopen(self.am_url + "/metrics", timeout=10).read().decode()
        def total(name):
            return sum(float(v) for v in re.findall(r'^%s\{integration="webhook"[^}]*\} (\S+)$' % name, text, re.M))
        return total("alertmanager_notifications_total"), total("alertmanager_notifications_failed_total")

    def until(self, predicate, timeout=60):
        deadline = time.time() + timeout
        while time.time() < deadline:
            if predicate():
                return True
            time.sleep(1)
        return False

    def logs(self):
        return "\n".join(self.sh("docker", "logs", "--tail", "20", c, check=False) for c in self.containers)

    def down(self):
        for c in self.containers:
            subprocess.run(["docker", "rm", "-f", c], capture_output=True)
        subprocess.run(["docker", "network", "rm", self.net], capture_output=True)


def shipped(doc):
    pass


def previous(doc):
    # The shape before this fix: every alert in the POST and ntfy's built-in template.
    for cfg in ntfy_webhooks(doc):
        cfg["max_alerts"] = 0
        cfg["url"] = cfg["url"].replace("template=goat-alertmanager", "template=alertmanager")


@unittest.skipUnless(os.environ.get("REQUIRE_DOCKER") == "1" or (docker_ok() and shutil.which("ansible-playbook")),
                     "needs Docker and ansible-playbook (CI sets REQUIRE_DOCKER=1)")
class Delivery(unittest.TestCase):
    def setUp(self):
        self.d = tempfile.mkdtemp()
        os.chmod(self.d, 0o755)

    def tearDown(self):
        shutil.rmtree(self.d, ignore_errors=True)

    def run_case(self, case, mutate):
        h = Harness(case, self.d)
        self.addCleanup(h.down)
        h.up(mutate)
        h.fire()
        return h

    def test_bounded_group_is_delivered_as_one_notification(self):
        h = self.run_case("bounded", shipped)
        self.assertTrue(h.until(lambda: h.messages() or h.counters()[1] > 0),
                        "nothing reached ntfy and nothing failed\n" + h.logs())
        time.sleep(6)  # one more group interval: a second POST would show up now
        sent, failed = h.counters()
        self.assertEqual(failed, 0, "Alertmanager counted failed ntfy notifications\n" + h.logs())
        msgs = h.messages()
        self.assertEqual(len(msgs), 1, "one group must be one notification, got %d\n%s" % (len(msgs), msgs))
        m = msgs[0]
        self.assertEqual(m["title"], "[FIRING 15+] DeliveryStorm @ pasture")
        self.assertIn("+ 10 more firing", m["message"])
        self.assertIn("+ 25 more not included by Alertmanager (max_alerts)", m["message"])
        self.assertEqual(m["message"].count("\n- "), 4, "five alerts listed, then counts:\n" + m["message"])
        self.assertNotIn("generatorURL", m["message"])
        self.assertNotIn("http://prometheus", m["message"])

    def test_control_previous_shape_drops_the_page(self):
        h = self.run_case("control", previous)
        self.assertTrue(h.until(lambda: h.counters()[1] > 0 or h.messages()),
                        "control: nothing happened at all\n" + h.logs())
        self.assertEqual(h.messages(), [], "control: the previous shape delivered; the harness proves nothing")
        self.assertGreater(h.counters()[1], 0)


if __name__ == "__main__":
    unittest.main(verbosity=2)
