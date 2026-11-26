"""peering-standup.sh phase 6 keeps the package the agreement was composed from.

Phase 3 rewrites <work>/ours.canonical.json on every run, so phase 6 copies it to
<work>/ours.composed.json beside agreement.json once compose passes; a later phase
3 reads the offer the applied agreement holds from that copy (goat-federation
`init`, the measured_mtu cross-check). The binary avenue does the same
(crates/goatbuilder/tests/federate_rotate.rs).
"""
import os
import stat
import subprocess
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]


def run_phase_6(tmp_path, compose_rc):
    work = tmp_path / "work"
    work.mkdir()
    (work / "ours.canonical.json").write_text('{"sovereign_id": "xaa", "phase": 3}\n')
    (work / "theirs.canonical.json").write_text('{"sovereign_id": "xab"}\n')
    stub = tmp_path / "goatbuilder"
    # A stand-in goatbuilder: `federate compose ... --out <file>` writes the agreement.
    stub.write_text(
        "#!/bin/sh\nout=\nwhile [ $# -gt 0 ]; do [ \"$1\" = --out ] && out=$2; shift; done\n"
        f"[ {compose_rc} = 0 ] || exit {compose_rc}\n"
        "printf '{\"transport\": {\"mtu\": 1300}}\\n' > \"$out\"\n"
    )
    stub.chmod(stub.stat().st_mode | stat.S_IXUSR)
    env = {k: v for k, v in os.environ.items() if not k.startswith("FED_")}
    env.update(FED_WORK=str(work), GOATBUILDER_BIN=str(stub), FED_VERBS="goatbuilder", FED_NO_DIRECTORY="1")
    r = subprocess.run([str(REPO / "ops/scripts/peering-standup.sh"), "xab-peer", "--from", "6", "--to", "6"],
                       env=env, capture_output=True, text=True, timeout=120)
    return work, r


def test_phase_6_copies_our_package_beside_the_agreement(tmp_path):
    work, r = run_phase_6(tmp_path, 0)
    assert r.returncode == 0, r.stdout + r.stderr
    assert (work / "agreement.json").is_file()
    assert (work / "ours.composed.json").read_bytes() == (work / "ours.canonical.json").read_bytes()


def test_a_failed_compose_keeps_no_copy(tmp_path):
    work, r = run_phase_6(tmp_path, 20)
    assert r.returncode == 20, r.stdout + r.stderr
    assert not (work / "ours.composed.json").exists()
