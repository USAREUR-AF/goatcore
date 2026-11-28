"""mesh-enroll-host.yml makes the NetBird client trust the goatnet's root before `netbird up`.

A LAN site's management serves file certificates from the goatnet's own CA
(dashboard_cert_delivery_mode: file). A fresh host trusts only the public
roots, so its NetBird client failed the TLS handshake to management
("certificate signed by unknown authority") and `netbird up` timed out: a
goatlite founder that declares mesh_enrollment_url stopped at standup phase 12,
where the site's own hosts are enrolled.

The play now gives the root (ops/enrollment/ca/<founder site>/root-cert.pem) to
the NetBird client alone before it mints the key: a root-owned file in a
directory of its own, and a drop-in that adds that directory to the client's
SSL_CERT_DIR after the system roots; then the client restarts. The host's
system store is never changed, so no other process trusts the root. It does so
only where the management serves the goatnet's certificates and the root is in
the checkout, and not where netbird_api_ca is public, the switch the API
playbooks use.

Run: python3 -m pytest -q tests/test_mesh_enroll_trusts_goatnet_root.py
"""
import os
import tempfile
import unittest

import jinja2
import yaml

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PLAY = os.path.join(REPO, "ops/ansible/mesh-enroll-host.yml")
TRUST = "mesh-enroll | the NetBird client trusts the goatnet's root (its management serves the goatnet's own certificates)"
MINT = "mesh-enroll | mint a single-use setup key (one use, one hour)"
UP = "mesh-enroll | netbird up against the declared URL"


def plays():
    with open(PLAY, encoding="utf-8") as fh:
        return yaml.safe_load(fh)


def walk(tasks):
    for t in tasks or []:
        yield t
        for key in ("block", "rescue", "always", "tasks"):
            yield from walk(t.get(key))


def enrol_play():
    return next(p for p in plays() if any(t.get("name") == UP for t in walk(p.get("tasks"))))


def env():
    e = jinja2.Environment(undefined=jinja2.StrictUndefined, autoescape=jinja2.select_autoescape(default_for_string=False))
    e.tests["exists"] = os.path.exists
    e.filters["bool"] = lambda x: x if isinstance(x, bool) else str(x).strip().lower() in ("true", "yes", "1", "on")
    return e


def render(text, **v):
    return env().from_string(text).render(**v)


def applies(when, root_dir, **v):
    """Each `when` line of the trust block, as Ansible evaluates a list (all true)."""
    play = enrol_play()
    pv = dict(v, playbook_dir=os.path.join(root_dir, "ops/ansible"))
    site = render(play["vars"]["_me_ca_site"], **pv)
    pv["_me_ca_site"] = site
    pv["_me_ca_root"] = render(play["vars"]["_me_ca_root"], **pv)
    return all(render("{{ (" + w + ") | bool }}", **pv) == "True" for w in when), pv["_me_ca_root"]


class TrustsTheGoatnetRoot(unittest.TestCase):
    def test_the_client_alone_trusts_the_root_before_the_key_is_minted(self):
        tasks = list(walk(enrol_play()["tasks"]))
        names = [t.get("name") for t in tasks]
        self.assertIn(TRUST, names, "no step gives the NetBird client the goatnet's root")
        self.assertLess(names.index(TRUST), names.index(MINT))
        self.assertLess(names.index(MINT), names.index(UP))
        block = next(t for t in tasks if t.get("name") == TRUST)["block"]
        trust_dir = enrol_play()["vars"]["_me_trust_dir"]
        copies = [b["ansible.builtin.copy"] for b in block if "ansible.builtin.copy" in b]
        root = next(c for c in copies if c.get("src") == "{{ _me_ca_root }}")
        self.assertEqual((root["dest"], root["owner"], root["mode"]), ("{{ _me_trust_dir }}/goatnet-root.pem", "root", "0644"))
        dropin = next(c for c in copies if c["dest"].startswith("/etc/systemd/system/netbird.service.d/"))
        text = render(dropin["content"], ansible_managed="managed", _me_trust_dir=trust_dir)
        lines = text.splitlines()
        self.assertIn("Environment=SSL_CERT_DIR=/etc/ssl/certs:" + trust_dir, lines)
        self.assertLess(lines.index("[Service]"), lines.index("Environment=SSL_CERT_DIR=/etc/ssl/certs:" + trust_dir))
        restart = block[-1]
        self.assertEqual(restart["ansible.builtin.systemd"], {"name": "netbird", "state": "restarted", "daemon_reload": True})
        self.assertIn("_me_ca_copy is changed", restart["when"])
        self.assertIn("_me_ca_dropin is changed", restart["when"])

    def test_the_hosts_system_store_is_never_changed(self):
        with open(PLAY, encoding="utf-8") as fh:
            text = fh.read()
        self.assertNotIn("/usr/local/share/ca-certificates", text)
        self.assertNotIn("update-ca-certificates", text)
        for t in walk(enrol_play()["tasks"]):
            for mod in ("ansible.builtin.copy", "ansible.builtin.file"):
                dest = str((t.get(mod) or {}).get("dest", (t.get(mod) or {}).get("path", "")))
                self.assertFalse(dest.startswith("/etc/ssl") or dest.startswith("/usr/share/ca-certificates"), dest)

    def test_only_where_the_management_serves_the_goatnets_own_certificates(self):
        when = next(t for t in walk(enrol_play()["tasks"]) if t.get("name") == TRUST)["when"]
        with tempfile.TemporaryDirectory() as d:
            ca = os.path.join(d, "ops/enrollment/ca/zagros")
            os.makedirs(ca)
            open(os.path.join(ca, "root-cert.pem"), "w").close()
            os.makedirs(os.path.join(d, "ops/ansible"))
            ok, root = applies(when, d, site_id="zagros")
            self.assertTrue(ok, "a LAN founder (no mode declared: file) trusts its root")
            self.assertTrue(root.endswith("/enrollment/ca/zagros/root-cert.pem"), root)
            ok, root = applies(when, d, site_id="beta", join_founder_site="zagros", dashboard_cert_delivery_mode="file")
            self.assertTrue(ok and root.endswith("/ca/zagros/root-cert.pem"), "a joining site's host trusts the founder's root")
            self.assertFalse(applies(when, d, site_id="zagros", dashboard_cert_delivery_mode="acme")[0], "an ACME site is left alone")
            self.assertFalse(applies(when, d, site_id="zagros", netbird_api_ca="public")[0], "netbird_api_ca public is left alone")
            self.assertFalse(applies(when, d, site_id="other")[0], "no root in the checkout: nothing is installed")
            self.assertFalse(applies(when, d)[0], "no site_id: nothing is installed, and nothing fails")


if __name__ == "__main__":
    unittest.main()
