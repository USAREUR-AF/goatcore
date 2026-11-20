"""Phase 2's two halves for a two-driver peering (ops/ansible/peering-prepare.yml).

`peering_prepare_half=edge` runs the edge's part alone on the edge (the federation
interface) and, with `peering_prepare_result`, writes its public key to that file on
the driver. `peering_prepare_half=core` runs the controller's part alone (the SPIRE
bundle export, delegated to the SPIRE server host) and never reaches the edge host,
which the controller cannot reach (docs/design/federation-two-drivers.md). Unset,
both run, as with one driver.
"""
import json
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
PLAY = REPO / "ops/ansible/peering-prepare.yml"


def require_ansible_playbook(case):
    """Skip without ansible-playbook, unless GOAT_TEST_REQUIRE_ANSIBLE=1 (CI sets
    it), where its absence fails the test instead of skipping it quietly."""
    if shutil.which("ansible-playbook"):
        return
    if os.environ.get("GOAT_TEST_REQUIRE_ANSIBLE") == "1":
        case.fail("GOAT_TEST_REQUIRE_ANSIBLE=1 but ansible-playbook is not on PATH")
    case.skipTest("ansible-playbook is required")


class PreparePlayHalves(unittest.TestCase):
    def setUp(self):
        require_ansible_playbook(self)
        self.tmp = Path(tempfile.mkdtemp(prefix="peering-prepare-halves-"))
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.bin = self.tmp / "bin"
        self.bin.mkdir()
        # A stand-in docker: `docker exec <c> <bin> bundle show -format spiffe`
        # prints a bundle, and every call is logged.
        self.calls = self.tmp / "calls"
        docker = self.bin / "docker"
        docker.write_text(f"#!/bin/sh\necho \"docker $*\" >> '{self.calls}'\necho '{{\"keys\": []}}'\n")
        docker.chmod(0o755)

    def play(self, edge_address, half=None, result=None, wg=False, play=PLAY):
        inv = self.tmp / "inventory.ini"
        # The edge is a host the run must not reach when only the core half
        # runs: a documentation address over ssh with a one-second timeout.
        inv.write_text(
            "[peering_edges]\n"
            f"edge-1 ansible_host={edge_address} ansible_connection={'local' if edge_address == 'local' else 'ssh'}"
            " ansible_ssh_common_args='-o ConnectTimeout=1 -o BatchMode=yes'\n"
            "[spire_servers]\nspire-1 ansible_connection=local\n"
        )
        work = self.tmp / "work"
        work.mkdir(exist_ok=True)
        extra = {
            "peering_wg_enabled": wg,
            "peering_wg_interface": "wg-fed9",
            "peering_spire_enabled": True,
            "peering_spire_server_host": "spire-1",
            "peering_spire_container": "spire-server",
            "fed_work": str(work),
            "ansible_become": False,
            "ansible_python_interpreter": shutil.which("python3"),
        }
        if half:
            extra["peering_prepare_half"] = half
        if result:
            extra["peering_prepare_result"] = str(result)
            extra["peering_prepare_peering"] = "xab"
        env = dict(os.environ, PATH=f"{self.bin}:{os.environ['PATH']}", ANSIBLE_NOCOLOR="1",
                   ANSIBLE_RETRY_FILES_ENABLED="0", ANSIBLE_LOCALHOST_WARNING="0",
                   ANSIBLE_HOST_KEY_CHECKING="0", ANSIBLE_BECOME="0")
        r = subprocess.run(
            ["ansible-playbook", "-i", str(inv), str(play), "-e", json.dumps(extra)],
            cwd=REPO / "ops/ansible", env=env, capture_output=True, text=True, timeout=180,
        )
        return r, work

    def test_the_core_half_exports_the_bundle_and_never_reaches_the_edge(self):
        # The federation interface is declared (peering_wg_enabled): only its
        # edge half would reach the edge, and the core half must not run it.
        r, work = self.play("192.0.2.1", half="core", wg=True)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertNotIn("UNREACHABLE", r.stdout)
        self.assertEqual((work / "ours.spiffe-bundle").read_text().strip(), '{"keys": []}')
        self.assertIn("bundle show -format spiffe", self.calls.read_text())

    def test_the_edge_half_runs_no_export(self):
        result = self.tmp / "edge" / "handoff/2/edge.json"
        r, work = self.play("local", half="edge", result=result)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertFalse(self.calls.exists(), "the edge half ran the SPIRE export")
        self.assertFalse((work / "ours.spiffe-bundle").exists())
        doc = json.loads(result.read_text())
        self.assertEqual(doc["schema"], "goatbuilder.phase2-edge/v1")
        self.assertEqual(doc["side"], "edge")
        self.assertEqual(doc["peering"], "xab")
        # peering_wg_enabled is off here: an edge with no federation interface.
        self.assertIsNone(doc["interface"])
        self.assertIsNone(doc["public_key"])

    def test_the_edge_half_writes_the_interfaces_public_key(self):
        # The play's wg branch on an edge with a federation interface: a stand-in
        # `wg` prints the key, and goat-peering-wg's prepare (packages, the key
        # under root) is a stand-in beside a copy of the play, so the play's own
        # tasks run unchanged and nothing needs root.
        wg = self.bin / "wg"
        wg.write_text(f"#!/bin/sh\necho \"wg $*\" >> '{self.calls}.wg'\n"
                      "[ \"$*\" = 'show wg-fed9 public-key' ] && { echo 'c3RhbmQtaW4tcHVibGljLWtleS1vZi13Zy1mZWQ5IT0='; exit 0; }\n"
                      "echo \"wg: nothing answers $*\" >&2; exit 1\n")
        wg.chmod(0o755)
        here = self.tmp / "play"
        (here / "roles/goat-peering-wg/tasks").mkdir(parents=True)
        shutil.copy(PLAY, here / PLAY.name)
        (here / "roles/goat-peering-spire").symlink_to(REPO / "ops/ansible/roles/goat-peering-spire")
        (here / "roles/goat-peering-wg/tasks/prepare.yml").write_text(
            "---\n- name: stand-in prepare\n  ansible.builtin.debug:\n    msg: stand-in goat-peering-wg prepare ran\n")
        result = self.tmp / "edge" / "handoff/2/edge.json"
        r, work = self.play("local", half="edge", result=result, wg=True, play=here / PLAY.name)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn("stand-in goat-peering-wg prepare ran", r.stdout)
        self.assertIn("wg show wg-fed9 public-key", Path(f"{self.calls}.wg").read_text())
        doc = json.loads(result.read_text())
        self.assertEqual(doc["interface"], "wg-fed9")
        self.assertEqual(doc["public_key"], "c3RhbmQtaW4tcHVibGljLWtleS1vZi13Zy1mZWQ5IT0=")
        self.assertFalse(self.calls.exists(), "the edge half ran the SPIRE export")

    def test_a_half_that_is_not_edge_or_core_is_refused(self):
        r, _ = self.play("local", half="both-of-them")
        self.assertNotEqual(r.returncode, 0, r.stdout)
        self.assertIn("peering_prepare_half", r.stdout + r.stderr)

    def test_unset_runs_both_as_with_one_driver(self):
        r, work = self.play("local")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertTrue((work / "ours.spiffe-bundle").is_file())


if __name__ == "__main__":
    unittest.main()
