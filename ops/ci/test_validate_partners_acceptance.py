# SPDX-License-Identifier: Apache-2.0
"""validate-partners.py accepts the `acceptance:` block acceptance_session.py propose
writes, refuses one that claims more than the grant or lacks its evidence, and only
notes an expired one (docs/design/federation-acceptance.md section 7)."""
import os
import subprocess
import sys
import textwrap

import pytest

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
VALIDATOR = os.path.join(ROOT, "ops", "federation", "validate-partners.py")
D = "sha256:" + "a" * 64

BASE = textwrap.dedent("""\
    sovereign_id: xab
    level: {level}
    partner_cn: peer-xab
    ingress:
      - "from/xab/**"
    egress:
      - "release/all/**"
    rationale: >-
      Synthetic declaration for the validator's acceptance tests. Not a peering.
""")

BLOCK = textwrap.dedent(f"""\
    acceptance:
      status: exchanging
      session: "ses-20261109T120000Z-0123456789ab"
      manifest_sha256: "{D}"
      agreement_sha256: "{D}"
      local_bundle_sha256: "{D}"
      peer_bundle_sha256: "{D}"
      combined_sha256: "{D}"
      verified_at: "2026-11-09T12:10:00Z"
      expires: "{{expires}}"
      approved_by:
        xaa: "Operator A"
        xab: "Operator B"
""")


def run(tmp_path, level="exchanging", block="", expires="2999-01-01"):
    d = tmp_path / "partners"
    d.mkdir()
    (d / "xab.yaml").write_text(BASE.format(level=level) + block.replace("{expires}", expires))
    return subprocess.run([sys.executable, VALIDATOR, "--partners", str(d)], capture_output=True, text=True)


def test_no_acceptance_block_passes(tmp_path):
    r = run(tmp_path, level="recognised")
    assert r.returncode == 0, r.stdout + r.stderr


def test_a_proposed_block_passes(tmp_path):
    r = run(tmp_path, block=BLOCK)
    assert r.returncode == 0, r.stdout + r.stderr


def test_an_expired_block_is_a_note_not_a_failure(tmp_path):
    r = run(tmp_path, block=BLOCK, expires="2020-01-01")
    assert r.returncode == 0, r.stdout
    assert "it is stale" in r.stdout, r.stdout


@pytest.mark.parametrize("level,block,why", [
    ("connected", BLOCK, "cannot claim more than the grant"),
    ("exchanging", BLOCK.replace('  combined_sha256: "%s"\n' % D, ""), "acceptance.combined_sha256 is required"),
    ("exchanging", BLOCK.replace("    xaa: \"Operator A\"\n", ""), "approved_by must name"),
    ("exchanging", BLOCK.replace("status: exchanging", "status: accepted"), "acceptance.status must be one of"),
    ("exchanging", BLOCK.replace('agreement_sha256: "%s"' % D, 'agreement_sha256: "abc"'), "must be sha256"),
    ("exchanging", BLOCK + "  level: exchanging\n", "not an acceptance field"),
])
def test_a_bad_block_is_refused(tmp_path, level, block, why):
    r = run(tmp_path, level=level, block=block)
    assert r.returncode == 1, r.stdout
    assert why in r.stdout, r.stdout
