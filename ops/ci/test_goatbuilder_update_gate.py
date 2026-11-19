"""Tests for ops/ci/goatbuilder-update-gate.py: the release gate that runs installed
goatbuilder releases' `update` against a candidate on a local stand-in surface.

Run: python3 -m pytest -q ops/ci/test_goatbuilder_update_gate.py
"""
import importlib.util
import json
import os
import stat
import sys
import textwrap
from pathlib import Path

import pytest

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("gate", HERE / "goatbuilder-update-gate.py")
gate = importlib.util.module_from_spec(spec)
sys.modules["gate"] = gate
spec.loader.exec_module(gate)

TAGS = [
    "goatbuilder/v0.4.1", "goatbuilder/v0.4.2", "goatbuilder/v0.4.3",
    "goatbuilder/v0.5.1", "goatbuilder/v0.5.3",
    "goatbuilder/v0.6.0", "goatbuilder/v0.6.1",
    "goatbuilder/v0.7.0", "goatbuilder/v0.7.1", "goatbuilder/v0.7.2", "goatbuilder/v0.7.3",
    "goatbuilder/v1.0.0-rc.1", "goatbuilder/v1.0.0", "goatbuilder/v1.1.0", "goatbuilder/v1.1.2",
    "other/v9.9.9",
]


# ---- which installed releases the gate runs -------------------------------------------

def test_last_release_of_each_minor_from_the_floor_below_the_candidate():
    got = gate.select_installed(TAGS, candidate="0.7.3", floor="0.4")
    assert got == ["0.4.3", "0.5.3", "0.6.1", "0.7.2"]


def test_the_floor_drops_older_minors():
    assert gate.select_installed(TAGS, candidate="0.7.3", floor="0.6") == ["0.6.1", "0.7.2"]


def test_prereleases_and_the_candidate_itself_are_never_installed_versions():
    got = gate.select_installed(TAGS, candidate="1.0.0", floor="0.7")
    assert got == ["0.7.3"]
    assert "1.0.0-rc.1" not in got


def test_from_1x_on_only_the_last_release_of_each_major_counts():
    got = gate.select_installed(TAGS, candidate="1.2.0", floor="0.7")
    assert got == ["0.7.3", "1.1.2"]


def test_a_candidate_that_is_not_semver_is_refused():
    with pytest.raises(ValueError):
        gate.select_installed(TAGS, candidate="next", floor="0.4")


# ---- the declared file ----------------------------------------------------------------

def write_declared(tmp_path, body):
    p = tmp_path / "gate.yaml"
    p.write_text(textwrap.dedent(body))
    return p


def test_a_declared_refusal_needs_a_code_a_why_and_the_fix_it_points_at(tmp_path):
    p = write_declared(tmp_path, """
        floor: "0.4"
        refuses:
          - version: "0.4.1"
            code: GB-074
            why: built before the release repository moved; its compiled-in signer is the previous release repository
            fix: goatbuilder explain GB-074
    """)
    d = gate.load_declared(p)
    assert d.floor == "0.4"
    assert d.refuses["0.4.1"].code == "GB-074"


@pytest.mark.parametrize("missing", ["code", "why", "fix"])
def test_a_declared_refusal_missing_a_field_is_rejected(tmp_path, missing):
    entry = {"version": "0.4.1", "code": "GB-074", "why": "x", "fix": "goatbuilder explain GB-074"}
    del entry[missing]
    p = tmp_path / "gate.yaml"
    p.write_text(json.dumps({"floor": "0.4", "refuses": [entry]}))
    with pytest.raises(gate.DeclaredError):
        gate.load_declared(p)


def test_a_declared_refusal_whose_fix_does_not_name_its_code_is_rejected(tmp_path):
    p = tmp_path / "gate.yaml"
    p.write_text(json.dumps({"floor": "0.4", "refuses": [
        {"version": "0.4.1", "code": "GB-074", "why": "x", "fix": "reinstall"}]}))
    with pytest.raises(gate.DeclaredError):
        gate.load_declared(p)


def test_the_repository_declared_file_loads():
    gate.load_declared(HERE / "goatbuilder-update-gate.yaml")


# ---- reading one run ------------------------------------------------------------------

def test_an_install_is_read_from_the_version_the_binary_then_reports():
    r = gate.read_result(output="update: ok (6 checks)\n", after_version="goatbuilder 0.7.3\n")
    assert (r.outcome, r.version, r.code) == ("installed", "0.7.3", None)


def test_a_refusal_is_read_as_its_first_code():
    r = gate.read_result(output="GB-074 signature refused ... GB-075\n", after_version="goatbuilder 0.4.1\n")
    assert (r.outcome, r.version, r.code) == ("refused", "0.4.1", "GB-074")


def test_a_result_keeps_no_subject_text():
    r = gate.read_result(output="GB-074 none of the expected identities matched https://github.com/some-org/x\n",
                         after_version="goatbuilder 0.4.1\n")
    assert "github.com" not in gate.render_row("0.4.1", r, "installed 0.7.3", True)


# ---- the verdict ----------------------------------------------------------------------

def test_an_installed_version_that_reaches_the_candidate_passes():
    r = gate.Result("installed", "0.7.3", None)
    assert gate.judge("0.7.2", r, candidate="0.7.3", declared={}) == (True, "installed 0.7.3")


def test_an_undeclared_refusal_fails_the_gate():
    r = gate.Result("refused", "0.6.1", "GB-074")
    ok, _ = gate.judge("0.6.1", r, candidate="0.7.3", declared={})
    assert not ok


def test_a_declared_refusal_with_its_code_passes():
    d = {"0.4.1": gate.Refusal("GB-074", "why", "goatbuilder explain GB-074")}
    ok, want = gate.judge("0.4.1", gate.Result("refused", "0.4.1", "GB-074"), candidate="0.7.3", declared=d)
    assert ok and want == "refused GB-074"


def test_a_declared_refusal_with_an_unexpected_code_fails_the_gate():
    d = {"0.4.1": gate.Refusal("GB-074", "why", "goatbuilder explain GB-074")}
    ok, _ = gate.judge("0.4.1", gate.Result("refused", "0.4.1", "GB-073"), candidate="0.7.3", declared=d)
    assert not ok


def test_a_declared_refusal_that_now_installs_fails_so_the_list_only_shrinks():
    d = {"0.4.1": gate.Refusal("GB-074", "why", "goatbuilder explain GB-074")}
    ok, _ = gate.judge("0.4.1", gate.Result("installed", "0.7.3", None), candidate="0.7.3", declared=d)
    assert not ok


def test_an_install_that_reports_another_version_fails():
    ok, _ = gate.judge("0.7.2", gate.Result("installed", "0.7.2", None), candidate="0.7.3", declared={})
    assert not ok


# ---- end to end, with stand-in binaries ----------------------------------------------

def fake_binary(path, *, installs_to=None, refuses=None, starts="0.7.2"):
    """A stand-in for an installed goatbuilder: `--version` reports `starts`; `update`
    either replaces itself with one reporting `installs_to`, or prints `refuses`."""
    path.write_text(textwrap.dedent(f"""\
        #!/bin/sh
        if [ "$1" = --version ]; then echo "goatbuilder {starts}"; exit 0; fi
        if [ "$1" = update ]; then
          [ -n "$GOATBUILDER_UPDATE_URL" ] || {{ echo "GB-072 no surface"; exit 4; }}
          curl -fsS "$GOATBUILDER_UPDATE_URL/version.json" >/dev/null || {{ echo "GB-073 unread"; exit 4; }}
          {'echo "' + refuses + ' refused"; exit 4' if refuses else
           'printf "#!/bin/sh\\necho goatbuilder ' + str(installs_to) + '\\n" > "$0"; echo "update: ok"; exit 0'}
        fi
        exit 2
    """))
    path.chmod(path.stat().st_mode | stat.S_IXUSR)
    return path


@pytest.fixture
def candidate(tmp_path):
    d = tmp_path / "candidate"
    d.mkdir()
    for f in ("goatbuilder-linux-amd64", "goatbuilder-linux-amd64.sha256", "SHA256SUMS", "SHA256SUMS.cosign-bundle"):
        (d / f).write_text("x\n")
    return d


def test_run_passes_when_every_installed_release_reaches_the_candidate(tmp_path, candidate):
    bins = {"0.6.1": fake_binary(tmp_path / "b061", installs_to="0.7.3", starts="0.6.1"),
            "0.7.2": fake_binary(tmp_path / "b072", installs_to="0.7.3")}
    rc, rows = gate.run(candidate, "0.7.3", bins, declared={}, work=tmp_path / "w")
    assert rc == 0, rows
    assert all(r.startswith(("0.6.1", "0.7.2")) for r in rows)


def test_run_fails_when_one_refuses_undeclared(tmp_path, candidate):
    bins = {"0.7.2": fake_binary(tmp_path / "b072", refuses="GB-074")}
    rc, rows = gate.run(candidate, "0.7.3", bins, declared={}, work=tmp_path / "w")
    assert rc == 1, rows


def test_run_serves_version_json_naming_the_candidate(tmp_path, candidate):
    gate.stage(candidate, "0.7.3", tmp_path / "srv")
    assert json.loads((tmp_path / "srv" / "goatbuilder" / "version.json").read_text()) == {"version": "0.7.3"}


def test_stage_refuses_a_candidate_missing_a_signed_file(tmp_path, candidate):
    (candidate / "SHA256SUMS.cosign-bundle").unlink()
    with pytest.raises(gate.CandidateError):
        gate.stage(candidate, "0.7.3", tmp_path / "srv")
