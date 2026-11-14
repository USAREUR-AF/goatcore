# SPDX-License-Identifier: Apache-2.0
"""The peering plays require the host firewall baseline on an edge (GB-279).

Every play the federation standup runs on a peering edge (phases 2, 11 and 12) checks
goat_host_firewall_baseline before it changes anything on the host: with the baseline off,
or not set (the role's default is false), the play refuses with GB-279, naming the host,
the setting and the fix; with it on, the play goes on to its next task.

The plays run for real here, on localhost with a local connection and no become, as the
one host of the peering_edges group. Nothing past the check is needed: the refusal is the
first task after fact gathering, and "goes on" is the play reaching its next task.
"""
import json
import os
import pathlib
import shutil
import subprocess
import sys

import pytest
import yaml

ANSIBLE = pathlib.Path(__file__).resolve().parents[3]

# The edge plays of a peering standup, by phase (crates/goatbuilder/src/federate/mod.rs).
EDGE_PLAYS = [
    "peering-prepare.yml",     # phase 2, the first change to the edge
    "peering-wg.yml",          # phase 11, wireguard
    "peering-edge.yml",        # phase 11, ipsec-fips
    "peering-zenoh-edge.yml",  # phase 12
]

# The core play keeps the baseline opt-in.
CORE_PLAYS = ["peering-republisher.yml"]

HOST = "edge-x"


def _is_require(task):
    inc = task.get("ansible.builtin.include_role") or task.get("include_role") or {}
    return inc.get("name") == "goat-host-firewall" and inc.get("tasks_from") == "require.yml"


def _playbook():
    found = shutil.which("ansible-playbook")
    if found:
        return found
    if os.environ.get("CI"):
        pytest.fail("ansible-playbook is not installed: the GB-279 refusal cannot be run")
    pytest.skip("ansible-playbook is not on PATH")


def _run(tmp_path, play, extra):
    inv = tmp_path / "inventory.ini"
    inv.write_text(
        "[peering_edges]\n"
        f"{HOST} ansible_connection=local ansible_python_interpreter={sys.executable}\n"
    )
    extra = dict(extra, ansible_become=False)
    env = dict(os.environ, ANSIBLE_NOCOLOR="1", ANSIBLE_FORCE_COLOR="0", ANSIBLE_RETRY_FILES_ENABLED="0")
    return subprocess.run(
        [_playbook(), "-i", str(inv), "-e", json.dumps(extra), play],
        cwd=ANSIBLE, env=env, capture_output=True, text=True, timeout=300,
    )


def _next_task_name(play):
    """The name of the first task after the check, as ansible prints it."""
    doc = yaml.safe_load((ANSIBLE / play).read_text())[0]
    pre = doc.get("pre_tasks") or []
    after = [t for t in pre if not _is_require(t)]
    if after:
        return after[0]["name"]
    return None


@pytest.mark.parametrize("play", EDGE_PLAYS)
def test_the_check_is_the_first_task_of_every_edge_play(play):
    doc = yaml.safe_load((ANSIBLE / play).read_text())[0]
    pre = doc.get("pre_tasks") or []
    assert pre and _is_require(pre[0]), (
        f"{play}: the first pre_task must include goat-host-firewall's require.yml, "
        "so the edge is refused before anything on it changes"
    )


@pytest.mark.parametrize("play", CORE_PLAYS)
def test_the_core_play_keeps_the_baseline_opt_in(play):
    doc = yaml.safe_load((ANSIBLE / play).read_text())[0]
    tasks = (doc.get("pre_tasks") or []) + (doc.get("tasks") or []) + (doc.get("post_tasks") or [])
    assert not any(_is_require(t) for t in tasks), f"{play}: a core host is not held to GB-279"


@pytest.mark.parametrize("play", EDGE_PLAYS)
@pytest.mark.parametrize("extra", [{"goat_host_firewall_baseline": False}, {}], ids=["off", "unset"])
def test_an_edge_with_the_baseline_off_is_refused(tmp_path, play, extra):
    r = _run(tmp_path, play, extra)
    out = r.stdout + r.stderr
    assert r.returncode != 0, out
    assert "goat-host-firewall refuses (GB-279)" in out, out
    assert HOST in out and "goat_host_firewall_baseline" in out, out
    nxt = _next_task_name(play)
    if nxt:
        assert f"TASK [{nxt}]" not in out, f"{play} went past the refusal:\n{out}"
    # Nothing but facts and the check ran before the refusal.
    assert "changed:" not in out, out


@pytest.mark.parametrize("play", EDGE_PLAYS)
def test_an_edge_with_the_baseline_on_goes_on(tmp_path, play):
    r = _run(tmp_path, play, {"goat_host_firewall_baseline": True})
    out = r.stdout + r.stderr
    assert "refuses (GB-279)" not in out, out
    nxt = _next_task_name(play)
    if nxt:
        assert f"TASK [{nxt}]" in out, f"{play} did not reach its next task:\n{out}"
    else:
        # No later pre_task: the play reached its roles or tasks.
        assert "TASK [" in out.split("an edge requires the host firewall baseline", 1)[-1], out
