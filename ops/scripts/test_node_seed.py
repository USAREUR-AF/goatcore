#!/usr/bin/env python3
"""Tests for node-seed.py (the autoinstall seed per node class, sink 5).

Run: python3 ops/scripts/test_node_seed.py
"""
import importlib.util
import json
import os
import stat
import subprocess
import tempfile
import unittest
from pathlib import Path

import yaml

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
spec = importlib.util.spec_from_file_location("ns", HERE / "node-seed.py")
ns = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ns)
KEYS = ["ssh-ed25519 AAAAtestkey test@example"]


class Build(unittest.TestCase):
    def test_storage_is_luks_on_lvm_with_the_hosts_passphrase(self):
        ud, meta, _ = ns.build(REPO, "demo", "demo-core-a", "x86-tpm2", "pass-1", KEYS, "")
        st = ud["autoinstall"]["storage"]["layout"]
        self.assertEqual((st["name"], st["password"]), ("lvm", "pass-1"))
        self.assertEqual(st["match"], {"path": "/dev/nvme*", "size": "largest"})
        self.assertIn("local-hostname: demo-core-a", meta)

    def test_packages_are_the_class_plus_the_role_lists(self):
        ud, _, _ = ns.build(REPO, "demo", "h", "x86-tpm2", "p", KEYS, "")
        pk = ud["autoinstall"]["packages"]
        luks = yaml.safe_load((REPO / "ops/ansible/roles/luks-unlock/defaults/main.yml").read_text())
        for p in luks["luks_packages_tpm2_initramfs"] + luks["luks_packages_base"] + ["tang", "chrony"]:
            self.assertIn(p, pk)
        self.assertEqual(len(pk), len(set(pk)), "each package once")

    def test_the_cache_is_the_apt_proxy_only_when_given(self):
        with_cache, _, _ = ns.build(REPO, "demo", "h", "x86-tpm2", "p", KEYS, "10.1.2.3")
        self.assertEqual(with_cache["autoinstall"]["apt"], {"proxy": "http://10.1.2.3:3142"})
        without, _, _ = ns.build(REPO, "demo", "h", "x86-tpm2", "p", KEYS, "")
        self.assertNotIn("apt", without["autoinstall"])

    def test_day_one_access_is_key_only_with_sudo(self):
        ud, _, _ = ns.build(REPO, "demo", "h", "x86-tpm2", "p", KEYS, "", user="first-day")
        ai = ud["autoinstall"]
        self.assertEqual(ai["ssh"], {"install-server": True, "allow-pw": False})
        u = ai["user-data"]["users"][0]
        self.assertEqual((u["name"], u["lock_passwd"], u["sudo"], u["ssh_authorized_keys"]), ("first-day", True, "ALL=(ALL) NOPASSWD:ALL", KEYS))

    def test_the_account_and_state_dir_are_what_the_checkout_declares(self):
        with tempfile.TemporaryDirectory() as d:
            self.assertEqual(ns.seed_user(Path(d)), "goat")
            self.assertEqual(ns.state_dir(Path(d)), Path(os.path.expanduser("~/.goat/state")))
            cfg = Path(d) / "ops/goatbuilder"
            cfg.mkdir(parents=True)
            (cfg / "goatbuilder.yaml").write_text("deploy:\n  ssh_user: ops\n  state_dir: /srv/state\n")
            self.assertEqual(ns.seed_user(Path(d)), "ops")
            self.assertEqual(ns.state_dir(Path(d)), Path("/srv/state"))
        ud, _, _ = ns.build(REPO, "demo", "h", "x86-tpm2", "p", KEYS, "")
        self.assertEqual(ud["autoinstall"]["user-data"]["users"][0]["name"], ns.seed_user(REPO))

    def test_the_marker_names_the_seed_and_never_the_passphrase(self):
        a, _, ma = ns.build(REPO, "demo", "h", "x86-tpm2", "secret-one", KEYS, "")
        b, _, mb = ns.build(REPO, "demo", "h", "x86-tpm2", "secret-two", KEYS, "")
        self.assertEqual(ma["seed_sha256"], mb["seed_sha256"])
        late = "\n".join(a["autoinstall"]["late-commands"])
        self.assertIn(ma["seed_sha256"], late)
        self.assertNotIn("secret-one", late)

    def test_a_disk_serial_replaces_the_class_match(self):
        ud, _, _ = ns.build(REPO, "demo", "h", "x86-tpm2", "p", KEYS, "", disk_serial="S4EVNX0N")
        self.assertEqual(ud["autoinstall"]["storage"]["layout"]["match"], {"serial": "S4EVNX0N"})

    def test_an_unknown_class_is_refused(self):
        with self.assertRaises(ValueError):
            ns.build(REPO, "demo", "h", "no-such-class", "p", KEYS, "")


class Cli(unittest.TestCase):
    def run_cli(self, env_extra, *args):
        env = {k: v for k, v in os.environ.items() if not k.endswith("_LUKS_PASS") and k != "PLATFORM_ADMIN_SSH_PUBKEYS"}
        env.update(env_extra)
        return subprocess.run(["python3", str(HERE / "node-seed.py"), "demo", "--host", "demo-core-a", "--class", "x86-tpm2", *args],
                              env=env, capture_output=True, text=True)

    def test_no_passphrase_is_refused_naming_the_variable(self):
        r = self.run_cli({"PLATFORM_ADMIN_SSH_PUBKEYS": KEYS[0]})
        self.assertEqual(r.returncode, 1)
        self.assertIn("DEMO_CORE_A_LUKS_PASS", r.stderr)

    def test_an_empty_roster_is_refused(self):
        r = self.run_cli({"DEMO_CORE_A_LUKS_PASS": "p"})
        self.assertEqual(r.returncode, 1)
        self.assertIn("PLATFORM_ADMIN_SSH_PUBKEYS", r.stderr)

    def test_writes_both_files_0600_and_valid_cloud_config(self):
        with tempfile.TemporaryDirectory() as d:
            r = self.run_cli({"DEMO_CORE_A_LUKS_PASS": "p4ss", "PLATFORM_ADMIN_SSH_PUBKEYS": KEYS[0]}, "--out", d)
            self.assertEqual(r.returncode, 0, r.stderr)
            for name in ("user-data", "meta-data"):
                self.assertEqual(stat.S_IMODE(os.stat(Path(d) / name).st_mode), 0o600)
            text = (Path(d) / "user-data").read_text()
            self.assertTrue(text.startswith("#cloud-config\n"))
            self.assertEqual(yaml.safe_load(text)["autoinstall"]["version"], 1)
            # The late commands, run by sh against a stand-in /target, write the marker.
            for c in yaml.safe_load(text)["autoinstall"]["late-commands"]:
                subprocess.run(["sh", "-c", c.replace("/target", d + "/target")], check=True)
            marker = json.loads((Path(d) / "target/etc/goat/node-seed.json").read_text())
            self.assertEqual((marker["class"], marker["host"], marker["unlock_class"]), ("x86-tpm2", "demo-core-a", "tpm2"))


if __name__ == "__main__":
    unittest.main(verbosity=1)
