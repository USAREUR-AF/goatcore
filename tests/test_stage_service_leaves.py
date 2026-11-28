"""stage-service-leaves.yml stages each service's leaf where the host's group_vars
expect it (<service>_cert_dir, else the role's default), and fails closed.

A core join on hardware found the crawler, node-net and ntp leaves not where
their group_vars pointed: the play staged each leaf to a fixed directory, and
had no entry at all for node-net-health or ntp-health-pub. Each case runs the
real play on localhost against an inventory whose group_vars point every
service's cert directory into the test's own directory, with leaves standing
in for the CA ceremony's output. Become is off and the files are owned by
whoever runs the test; nothing is minted and nothing outside the test's
directory is written.

Checked:
  - every service in the play's map is staged at its group_vars directory,
    under the file names its role reads;
  - each map default is the role's own default, for the roles in this tree;
  - a leaf missing from the source directory is refused;
  - two services that resolve to the same file are refused, naming both;
  - under defaults the crawler reads fabric-health's leaf: it is not staged
    over it, and the run says so;
  - no CA root is staged into a service's cert directory.

Run: python3 -m pytest tests/test_stage_service_leaves.py
"""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

import yaml

ROOT = Path(__file__).resolve().parents[1]
PLAY = ROOT / "ops/ansible/stage-service-leaves.yml"
GROUP = "zagros_hosts"

# The services the play stages, the role variable naming each one's directory,
# and the file names its role reads.
EXPECTED = {
    "fabric-health": ("fabric_health_cert_dir", "fabric-health"),
    "mesh-health": ("mesh_health_cert_dir", "mesh-health"),
    "presence-health": ("presence_health_cert_dir", "presence-health"),
    "time-health": ("time_health_cert_dir", "time-health"),
    "peer-health": ("peer_health_cert_dir", "peer-health"),
    "consistency-health": ("consistency_health_cert_dir", "consistency-health"),
    "consistency-console": ("consistency_console_cert_dir", "consistency-console"),
    "panoscope": ("panoscope_cert_dir", "panoscope"),
    "fabric-crawler": ("fabric_crawler_cert_dir", "fabric-crawler"),
    "goat-topology": ("goat_topology_cert_dir", "goat-topology"),
    "audit-subscriber": ("audit_subscriber_cert_dir", "audit-subscriber"),
    "node-net-health": ("node_net_health_cert_dir", "node-net-health"),
    "ntp-health-pub": ("ntp_health_pub_cert_dir", "ntp-health-pub"),
}


def require_ansible_playbook(case):
    if shutil.which("ansible-playbook"):
        return
    if os.environ.get("GOAT_TEST_REQUIRE_ANSIBLE") == "1":
        case.fail("GOAT_TEST_REQUIRE_ANSIBLE=1 but ansible-playbook is not on PATH")
    case.skipTest("ansible-playbook is required")


def play_map():
    return yaml.safe_load(PLAY.read_text())[0]["vars"]["service_leaf_map"]


class StageServiceLeaves(unittest.TestCase):

    def run_play(self, group_vars, leaves=None, extra=()):
        """Run the play against localhost in GROUP with group_vars. `leaves` is the
        set of services whose minted pair sits in the source directory (all by
        default). Returns (result, the test's directory)."""
        require_ansible_playbook(self)
        d = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, d)
        src = d / "minted"
        src.mkdir()
        for svc in (EXPECTED if leaves is None else leaves):
            (src / f"{svc}-cert.pem").write_text(f"CERT {svc}\n")
            (src / f"{svc}-key.pem").write_text(f"KEY {svc}\n")
        inv = d / "inventory"
        (inv / "group_vars" / GROUP).mkdir(parents=True)
        (inv / "hosts.ini").write_text(f"[{GROUP}]\nlocalhost ansible_connection=local\n")
        (inv / "group_vars" / GROUP / "vars.yml").write_text(
            yaml.safe_dump({k: v.format(d=d) if isinstance(v, str) else v for k, v in group_vars.items()}))
        env = dict(os.environ, ANSIBLE_NOCOLOR="1", ANSIBLE_LOCALHOST_WARNING="False",
                   ANSIBLE_BECOME="False", ANSIBLE_RETRY_FILES_ENABLED="False")
        env.pop("ANSIBLE_CONFIG", None)
        r = subprocess.run(
            ["ansible-playbook", "-i", str(inv / "hosts.ini"), str(PLAY),
             "-e", f"target={GROUP}", "-e", f"leaf_src_dir={src}",
             "-e", "ansible_become=false", "-e", f"leaf_owner={os.getuid()}", *extra],
            cwd=d, env=env, capture_output=True, text=True, timeout=300)
        return r, d

    def every_dir_in_the_test(self):
        """group_vars that put every service's directory under the test's own."""
        gv = {var: "{d}/certs/" + svc for svc, (var, _) in EXPECTED.items()}
        return gv

    def test_the_map_is_the_expected_services(self):
        self.assertEqual(sorted(play_map()), sorted(EXPECTED))

    def test_each_default_is_the_roles_own(self):
        for svc, entry in play_map().items():
            defaults = ROOT / "ops/ansible/roles" / svc / "defaults/main.yml"
            if not defaults.is_file():
                continue  # the role is not in this tree (goat-topology)
            with self.subTest(svc):
                d = yaml.safe_load(defaults.read_text())
                self.assertEqual(entry["dir_var"], EXPECTED[svc][0])
                self.assertEqual(entry["dir"], d[entry["dir_var"]])
                for kind in ("cert", "key"):
                    if f"{kind}_var" in entry:
                        self.assertEqual(entry[kind], d[entry[f"{kind}_var"]])

    def test_each_leaf_is_staged_at_its_group_vars_directory(self):
        gv = self.every_dir_in_the_test()
        # The crawler reads its own leaf when its group_vars name its own files.
        gv["fabric_crawler_cert_file"] = "fabric-crawler-cert.pem"
        gv["fabric_crawler_key_file"] = "fabric-crawler-key.pem"
        r, d = self.run_play(gv)
        self.assertEqual(r.returncode, 0, r.stdout[-4000:] + r.stderr[-2000:])
        pm = play_map()
        for svc in EXPECTED:
            with self.subTest(svc):
                here = d / "certs" / svc
                # The file names: a cert_var/key_var from group_vars, else the
                # entry's own cert/key (panoscope), else <service>-cert.pem.
                entry = pm[svc]
                cert = gv.get(entry.get("cert_var", ""), entry.get("cert", f"{svc}-cert.pem"))
                key = gv.get(entry.get("key_var", ""), entry.get("key", f"{svc}-key.pem"))
                self.assertEqual((here / cert).read_text(), f"CERT {svc}\n")
                self.assertEqual((here / key).read_text(), f"KEY {svc}\n")
                self.assertEqual(oct((here / key).stat().st_mode & 0o777), "0o400")
                # No CA root goes into a service's cert directory.
                self.assertFalse(any("ca-root" in p.name for p in here.iterdir()))

    def test_a_missing_leaf_is_refused(self):
        leaves = [s for s in EXPECTED if s != "node-net-health"]
        r, d = self.run_play(self.every_dir_in_the_test(), leaves=leaves)
        self.assertNotEqual(r.returncode, 0)
        refusal = [l for l in r.stdout.splitlines() if "no leaf in" in l]
        self.assertTrue(refusal and "node-net-health" in " ".join(refusal), r.stdout[-3000:])
        self.assertFalse((d / "certs" / "mesh-health").exists(), "a leaf was staged before the refusal")

    def test_two_services_on_one_file_are_refused(self):
        gv = self.every_dir_in_the_test()
        gv["fabric_crawler_cert_dir"] = "{d}/certs/mesh-health"
        gv["fabric_crawler_cert_file"] = "mesh-health-cert.pem"
        gv["fabric_crawler_key_file"] = "mesh-health-key.pem"
        r, d = self.run_play(gv)
        self.assertNotEqual(r.returncode, 0)
        refusal = [l for l in r.stdout.splitlines() if "same file" in l]
        self.assertTrue(refusal, r.stdout[-3000:])
        self.assertIn("fabric-crawler", " ".join(refusal))
        self.assertIn("mesh-health", " ".join(refusal))
        self.assertFalse((d / "certs" / "mesh-health").exists(), "a leaf was staged before the refusal")
        # Staging the crawler alone still may not overwrite mesh-health's file.
        r, d = self.run_play(gv, extra=("-e", '{"leaf_only": ["fabric-crawler"]}'))
        self.assertNotEqual(r.returncode, 0)
        self.assertTrue([l for l in r.stdout.splitlines() if "same file" in l], r.stdout[-3000:])
        self.assertFalse((d / "certs" / "mesh-health").exists(), "the crawler's leaf was staged over mesh-health's")

    def test_under_defaults_the_crawler_shares_fabric_healths_leaf(self):
        gv = self.every_dir_in_the_test()
        # The crawler's role default: fabric-health's directory and file names.
        gv["fabric_crawler_cert_dir"] = "{d}/certs/fabric-health"
        r, d = self.run_play(gv)
        self.assertEqual(r.returncode, 0, r.stdout[-4000:] + r.stderr[-2000:])
        fh = d / "certs" / "fabric-health"
        self.assertEqual((fh / "fabric-health-cert.pem").read_text(), "CERT fabric-health\n")
        self.assertFalse((fh / "fabric-crawler-cert.pem").exists())
        self.assertIn("fabric-crawler reads fabric-health's leaf", r.stdout)

    def test_a_leaf_only_run_is_not_refused_by_the_crawler_sharing_a_leaf(self):
        # Under defaults the crawler reads fabric-health's leaf. A run staging
        # only goat-topology must still set the crawler aside, or the
        # one-file-one-reader check over the whole map refuses the run.
        gv = self.every_dir_in_the_test()
        gv["fabric_crawler_cert_dir"] = "{d}/certs/fabric-health"
        r, d = self.run_play(gv, extra=("-e", '{"leaf_only": ["goat-topology"]}'))
        self.assertEqual(r.returncode, 0, r.stdout[-4000:] + r.stderr[-2000:])
        self.assertEqual((d / "certs" / "goat-topology" / "goat-topology-cert.pem").read_text(),
                         "CERT goat-topology\n")
        self.assertFalse((d / "certs" / "fabric-health").exists(), "only goat-topology is staged")

    def test_audit_subscriber_reads_its_ca_root_from_the_router(self):
        # Row 7 places no trust anchor: the audit subscriber reads the goatnet's
        # CA root from the router's TLS directory, not from its own cert dir.
        defaults = yaml.safe_load((ROOT / "ops/ansible/roles/audit-subscriber/defaults/main.yml").read_text())
        self.assertRegex(defaults["audit_subscriber_ca_root_src"], r"^/etc/\{\{ *zenoh_stack.*\}\}/tls/ca-root-cert\.pem$")
        tasks = (ROOT / "ops/ansible/roles/audit-subscriber/tasks/main.yml").read_text()
        self.assertIn("{{ audit_subscriber_ca_root_src }}", tasks)
        self.assertNotIn("audit_subscriber_cert_dir }}/ca-root", tasks)


if __name__ == "__main__":
    unittest.main()
