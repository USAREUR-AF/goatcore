"""goat-xf-pipe-deploy.yml refuses before it installs anything.

The pipe's play (gate J24) unpacks a release archive as root and runs its
program as a service, from a file the release-file role put in place. Two holds
come before that, and each is run here for real on localhost, with become off,
against the test's own directories (nothing outside them is written, nothing is
installed):

  - a goatnet that supplies no conversion, one missing a field, one whose metrics
    listener is every interface or not <address>:<port>, one holding a control
    character, or an empty Zenoh configuration, is refused at the first task with
    GB-396, and the release-file role never runs;
  - a file that `current` names is unpacked only when it is the verified
    goat-xf-pipe-zenoh archive, in the directory named for its own SHA-256, and
    neither it nor a directory above it can be changed by anyone but its owner:
    another name, or a file its group can write, is refused before it is unpacked;
    the verified file goes on to be unpacked.

ops/e2e/pipe-release.sh deploys the published releases to a VM, to their health
check, through both drivers.

Run: python3 -m pytest tests/test_goat_xf_pipe_play.py
"""
import hashlib
import json
import os
from pathlib import Path
import shutil
import sys
import subprocess
import tempfile
import unittest

import yaml

ROOT = Path(__file__).resolve().parents[1]
PLAY = ROOT / "ops/ansible/goat-xf-pipe-deploy.yml"
GROUP = "zagros_hosts"
PIN = "example-org/example-tool@xf-pipe-v0.1.6@sha256:" + "a" * 64
CONFIG = {"from_format": "cot-xml", "to_format": "geojson", "in": "zenoh:capture/cot/**",
          "out": "zenoh:converted/geojson", "metrics": "127.0.0.1:9464"}


def require_ansible_playbook(case):
    if shutil.which("ansible-playbook"):
        return
    if os.environ.get("GOAT_TEST_REQUIRE_ANSIBLE") == "1":
        case.fail("GOAT_TEST_REQUIRE_ANSIBLE=1 but ansible-playbook is not on PATH")
    case.skipTest("ansible-playbook is required")


class PipePlay(unittest.TestCase):

    def run_play(self, group_vars, file_name=None, mode="0644"):
        """Run the play on localhost in GROUP. With `file_name`, a stand-in
        package of that name is handed to the role as the verified file."""
        require_ansible_playbook(self)
        d = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, d)
        inv = d / "inventory"
        (inv / "group_vars" / GROUP).mkdir(parents=True)
        (inv / "hosts.ini").write_text(f"[{GROUP}]\nlocalhost ansible_connection=local\n")
        (inv / "group_vars" / GROUP / "vars.yml").write_text(yaml.safe_dump(group_vars))
        extra = {
            "target": GROUP, "ansible_become": False,
            "ansible_python_interpreter": "{{ ansible_playbook_python }}",
            "release_file_component": "goat-xf-pipe-zenoh",
            "release_file_root": str(d / "root"),
            "release_file_owner": subprocess.run(["id", "-un"], capture_output=True, text=True).stdout.strip(),
            "release_file_group": subprocess.run(["id", "-gn"], capture_output=True, text=True).stdout.strip(),
            "rollback_ledger_dir": str(d / "ledger"),
            "rollback_assembled_marker": str(d / "no-marker"),
            "release_file_mode": mode,
        }
        extra["rollback_ledger_owner"], extra["rollback_ledger_group"] = extra["release_file_owner"], extra["release_file_group"]
        if file_name:
            src = d / "src" / file_name
            src.parent.mkdir()
            src.write_bytes(b"not a package\n")
            sha = hashlib.sha256(src.read_bytes()).hexdigest()
            extra["release_file_ref"] = PIN
            extra["release_file_files"] = {a: {"src": str(src), "name": file_name, "sha256": sha} for a in ("amd64", "arm64")}
        env = dict(os.environ, ANSIBLE_NOCOLOR="1", ANSIBLE_LOCALHOST_WARNING="False",
                   ANSIBLE_BECOME="False", ANSIBLE_RETRY_FILES_ENABLED="False", ANSIBLE_CACHE_PLUGIN="memory")
        env.pop("ANSIBLE_CONFIG", None)
        r = subprocess.run(["ansible-playbook", "-i", str(inv / "hosts.ini"), str(PLAY), "-e", json.dumps(extra)],
                           cwd=d, env=env, capture_output=True, text=True, timeout=300)
        return r, d

    def test_no_configuration_is_refused_before_anything_is_installed(self):
        for gv in ({}, {"goat_xf_pipe": {}}, {"goat_xf_pipe": "from_format: x"},
                   {"goat_xf_pipe": {k: v for k, v in CONFIG.items() if k != "out"}},
                   {"goat_xf_pipe": {**CONFIG, "metrics": "0.0.0.0:9464"}},
                   {"goat_xf_pipe": {**CONFIG, "metrics": "9464"}},
                   {"goat_xf_pipe": {**CONFIG, "metrics": "0:9464"}},
                   {"goat_xf_pipe": {**CONFIG, "metrics": "203.0.113.5:9464"}},
                   {"goat_xf_pipe": {**CONFIG, "metrics": "127.0.0.1:99999"}},
                   {"goat_xf_pipe": {**CONFIG, "to_format": "geojson\\"}},
                   {"goat_xf_pipe": {**CONFIG, "in": "zenoh:a\nExecStartPre=/bin/true"}},
                   {"goat_xf_pipe": {**CONFIG, "zenoh_config": ""}}):
            with self.subTest(gv):
                r, d = self.run_play(gv, file_name="goat-xf-pipe-zenoh_0.1.6_linux_amd64.tar.gz")
                self.assertEqual(r.returncode, 2, r.stdout[-3000:])
                self.assertIn("GB-396 goat-xf-pipe-zenoh's conversion is not supplied", r.stdout)
                self.assertNotIn("release-file |", r.stdout, "the role ran before the refusal")
                self.assertFalse((d / "root").exists() or (d / "ledger").exists())

    def test_a_file_that_is_not_a_pipe_archive_is_not_unpacked(self):
        r, d = self.run_play({"goat_xf_pipe": CONFIG}, file_name="tool.tar.gz")
        self.assertNotEqual(r.returncode, 0, r.stdout[-3000:])
        self.assertIn("is not a verified goat-xf-pipe-zenoh archive", r.stdout, r.stdout[-3000:])
        self.assertNotIn("goat-xf-pipe-zenoh | 2 the archive and where it is unpacked", r.stdout.split("is not a verified")[1])
        # The role did put it in place: the hold is the play's own, after it.
        self.assertTrue((d / "root" / "goat-xf-pipe-zenoh" / "current").is_symlink())

    def test_the_verified_archive_passes_the_hold_and_goes_on_to_be_unpacked(self):
        # A stand-in under the archive's name: held and accepted; it stops after the
        # hold (this run is not root, so it cannot make the unpacked directory).
        r, d = self.run_play({"goat_xf_pipe": CONFIG}, file_name="goat-xf-pipe-zenoh_0.1.6_linux_amd64.tar.gz")
        self.assertNotEqual(r.returncode, 0)
        self.assertNotIn("is not a verified goat-xf-pipe-zenoh archive", r.stdout, r.stdout[-3000:])
        self.assertIn("TASK [goat-xf-pipe-zenoh | 2 the archive and where it is unpacked] ", r.stdout)

    def test_an_archive_someone_else_can_change_is_not_unpacked(self):
        r, d = self.run_play({"goat_xf_pipe": CONFIG}, file_name="goat-xf-pipe-zenoh_0.1.6_linux_amd64.tar.gz",
                             mode="0664")
        self.assertIn("or someone other than", r.stdout, r.stdout[-3000:])
        self.assertNotIn("goat-xf-pipe-zenoh | 2 the archive and where it is unpacked", r.stdout.split("or someone other than")[1])



class Unpack(unittest.TestCase):
    """ops/ansible/files/xf-pipe-unpack.py judges every member before it unpacks."""

    def run_unpack(self, rows, top="tool-1"):
        import io
        import tarfile
        d = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, d)
        a = d / "a.tar.gz"
        with tarfile.open(a, "w:gz") as t:
            for kind, name, data in rows:
                i = tarfile.TarInfo(name)
                if kind == "d":
                    i.type = tarfile.DIRTYPE
                    t.addfile(i)
                elif kind == "l":
                    i.type, i.linkname = tarfile.SYMTYPE, data
                    t.addfile(i)
                elif kind == "h":
                    i.type, i.linkname = tarfile.LNKTYPE, data
                    t.addfile(i)
                else:
                    b = data.encode()
                    i.size, i.mode = len(b), 0o755
                    t.addfile(i, io.BytesIO(b))
        script = ROOT / "ops/ansible/files/xf-pipe-unpack.py"
        import grp
        group = grp.getgrgid(os.getgid()).gr_name
        return subprocess.run([sys.executable, "-I", str(script), str(a), top, str(d / "pipe" / "sha"), group], capture_output=True, text=True), d

    GOOD = [("d", "tool-1", ""), ("f", "tool-1/goat-xf-pipe-zenoh", "x"), ("d", "tool-1/packs", ""), ("f", "tool-1/packs/a/pack.def", "p")]

    def test_a_member_that_is_not_a_plain_file_or_directory_under_its_top_is_refused(self):
        for row, words in [
            (("l", "tool-1/packs/etc", "/etc"), "not a plain file or directory"),
            (("h", "tool-1/hard", "tool-1/goat-xf-pipe-zenoh"), "not a plain file or directory"),
            (("f", "tool-1/../x", "x"), "not a plain one"),
            (("f", "/abs", "x"), "not a plain one"),
            (("f", "other/x", "x"), "outside its top directory"),
            (("f", "tool-1/packs/a/pack.def", "again"), "twice"),
        ]:
            with self.subTest(row):
                r, d = self.run_unpack(self.GOOD + [row])
                self.assertEqual(r.returncode, 1, r.stdout + r.stderr)
                self.assertIn(words, r.stderr)
                self.assertFalse((d / "pipe").exists(), "nothing is written before every member is judged")
        r, _ = self.run_unpack([("d", "tool-1", ""), ("f", "tool-1/README", "x")])
        self.assertIn("no program goat-xf-pipe-zenoh", r.stderr)

    def test_a_destination_that_is_not_roots_alone_is_refused(self):
        if os.geteuid() == 0:
            self.skipTest("run as root, the test's own directories are root's")
        r, _ = self.run_unpack(self.GOOD)
        self.assertEqual(r.returncode, 1)
        self.assertIn("is not root's alone", r.stderr)

    def test_the_tree_on_disk_may_carry_its_groups_read_and_no_more(self):
        import importlib.util
        spec = importlib.util.spec_from_file_location("xf_pipe_unpack", ROOT / "ops/ansible/files/xf-pipe-unpack.py")
        m = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(m)
        # As unpacked, and as the play leaves it: the owner's bits are what is held.
        for mode, held in [(0o700, 0o700), (0o750, 0o700), (0o710, 0o700), (0o600, 0o600), (0o640, 0o600)]:
            with self.subTest(oct(mode)):
                self.assertEqual(m.owner_mode(mode), held)
        # Anyone else, the group writing, or the group holding a bit the owner has not: a difference.
        for mode in [0o751, 0o754, 0o755, 0o644, 0o770, 0o660, 0o650, 0o604, 0o2750, 0o4750]:
            with self.subTest(oct(mode)):
                self.assertNotEqual(m.owner_mode(mode), 0o700 if mode & 0o100 else 0o600)

    def test_every_directory_an_archive_implies_is_given_the_group(self):
        import importlib.util
        import io
        import tarfile
        from unittest import mock
        spec = importlib.util.spec_from_file_location("xf_pipe_unpack", ROOT / "ops/ansible/files/xf-pipe-unpack.py")
        m = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(m)
        d = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, d)
        a = d / "a.tar.gz"
        # Files only: no member names tool-1, packs, packs/a or packs/a/b.
        with tarfile.open(a, "w:gz") as t:
            for name in ["tool-1/goat-xf-pipe-zenoh", "tool-1/packs/a/b/pack.def", "tool-1/packs/c/pack.def"]:
                i = tarfile.TarInfo(name)
                i.size, i.mode = 1, 0o755
                t.addfile(i, io.BytesIO(b"x"))
        into = d / "fresh"
        given = {}
        with mock.patch.object(m.os, "chown", lambda p, uid, gid: given.__setitem__(str(p), (uid, gid))):
            m.unpack(str(a), "tool-1", str(into), 4242)
        tree = into / "tool-1"
        dirs = [tree] + [p for p in tree.rglob("*") if p.is_dir()]
        self.assertEqual(len(dirs), 5)  # tool-1, packs, a, b, c
        for p in dirs + [p for p in tree.rglob("*") if p.is_file()]:
            with self.subTest(str(p.relative_to(d))):
                self.assertEqual(given.get(str(p)), (0, 4242), "root's and the group's")
        for p in dirs:
            with self.subTest(str(p.relative_to(d))):
                self.assertEqual(p.stat().st_mode & 0o7777, 0o700)


class GroupRead(unittest.TestCase):
    """The unpacker leaves the tree root's alone; the play then gives the service's group read."""

    def test_the_play_gives_the_services_group_read_and_no_one_else_anything(self):
        tasks = yaml.safe_load(PLAY.read_text())[0]["tasks"]
        names = [t.get("name", "") for t in tasks]
        unpack = names.index("goat-xf-pipe-zenoh | 2 unpack it, or hold what is there to it")
        current = names.index("goat-xf-pipe-zenoh | 2 current names it")
        grants = [i for i, t in enumerate(tasks) if unpack < i < current and "ansible.builtin.file" in t]
        self.assertEqual(len(grants), 1, "one task between the unpack and current sets the tree's modes")
        f = tasks[grants[0]]["ansible.builtin.file"]
        self.assertEqual(f["path"], "{{ _xp_dir }}/{{ _xp_sha }}")
        self.assertEqual((f["state"], f["recurse"], f["follow"], f["owner"], f["group"], f["mode"]),
                         ("directory", True, False, "root", "goat-xf-pipe", "u=rwX,g=rX,o="))


if __name__ == "__main__":
    unittest.main()
