# SPDX-License-Identifier: Apache-2.0
"""validate-partners.py accepts a well-formed `forwarder:` block and refuses a
malformed one (ADR 1218, docs/design/peering-forwarder.md §6)."""
import os
import subprocess
import sys
import textwrap

import pytest

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
VALIDATOR = os.path.join(ROOT, "ops", "federation", "validate-partners.py")

BASE = textwrap.dedent("""\
    sovereign_id: xaa
    level: recognised
    partner_cn: peer-xaa
    ingress:
      - "from/xaa/**"
    egress:
      - "release/all/**"
    rationale: >-
      Synthetic declaration for the validator's forwarder tests. Not a peering.
""")


def run(tmp_path, extra: str):
    d = tmp_path / "partners"
    d.mkdir()
    (d / "xaa.yaml").write_text(BASE + extra)
    return subprocess.run([sys.executable, VALIDATOR, "--partners", str(d)],
                          capture_output=True, text=True)


def test_no_forwarder_passes(tmp_path):
    r = run(tmp_path, "")
    assert r.returncode == 0, r.stdout + r.stderr


def test_wellformed_forwarder_passes(tmp_path):
    r = run(tmp_path, "forwarder:\n  operator: xab\n  placement: cloud-vm\n  endpoint: 192.0.2.20:51820\n")
    assert r.returncode == 0, r.stdout + r.stderr


@pytest.mark.parametrize("block,why", [
    ("forwarder: 192.0.2.20:51820\n", "must be a mapping"),
    ("forwarder:\n  operator: xab\n  placement: cloud-vm\n", "forwarder.endpoint is required"),
    ("forwarder:\n  operator: xab\n  placement: cloud-vm\n  endpoint: 192.0.2.20\n", "must be host:port"),
    ("forwarder:\n  operator: xab\n  placement: cloud-vm\n  endpoint: 192.0.2.20:70000\n", "must be host:port"),
    ("forwarder:\n  operator: xab\n  placement: somewhere\n  endpoint: 192.0.2.20:51820\n", "placement must be one of"),
    ("forwarder:\n  operator: Not A Sovereign\n  placement: owned-site\n  endpoint: 192.0.2.20:51820\n", "operator"),
    ("forwarder:\n  operator: xab\n  placement: owned-site\n  endpoint: 192.0.2.20:51820\n  private_key: x\n", "not a forwarder field"),
])
def test_malformed_forwarder_refused(tmp_path, block, why):
    r = run(tmp_path, block)
    assert r.returncode == 1, r.stdout
    assert why in r.stdout, r.stdout
