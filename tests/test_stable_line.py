"""Tests for ops/release/stable.py: when the stable branch may move (ADR 1231)."""
import importlib.util
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
_spec = importlib.util.spec_from_file_location("stable_line", ROOT / "ops/release/stable.py")
sl = importlib.util.module_from_spec(_spec)
sys.modules["stable_line"] = sl
_spec.loader.exec_module(sl)


def repo(tmp_path):
    """main: c1 - c2 - c3; side: c1 - x. Returns (path, {name: sha})."""
    r = str(tmp_path)

    def g(*a):
        return subprocess.run(["git", "-C", r, *a], check=True, capture_output=True, text=True).stdout.strip()

    g("init", "-q", "-b", "main")
    g("config", "user.email", "t@example.invalid")
    g("config", "user.name", "t")
    g("config", "commit.gpgsign", "false")
    g("config", "tag.gpgsign", "false")
    sha = {}
    for n in ("c1", "c2", "c3"):
        g("commit", "-q", "--allow-empty", "-m", n)
        sha[n] = g("rev-parse", "HEAD")
    g("checkout", "-q", "-b", "side", sha["c1"])
    g("commit", "-q", "--allow-empty", "-m", "x")
    sha["x"] = g("rev-parse", "HEAD")
    g("checkout", "-q", "main")
    return r, sha, g


def check(r, tag):
    return sl.refusals(r, tag, main="main", stable="stable")


def test_a_release_at_a_goatbuilder_release_on_main_moves_stable(tmp_path):
    r, sha, g = repo(tmp_path)
    g("tag", "goatbuilder/v0.6.0", sha["c2"])
    g("tag", "goatcore/v0.6.0", sha["c2"])
    assert check(r, "goatcore/v0.6.0") == []


def test_a_tag_of_another_shape_is_refused(tmp_path):
    r, sha, g = repo(tmp_path)
    g("tag", "goatcore/v0.6", sha["c2"])
    assert "not a whole-tree release tag" in check(r, "goatcore/v0.6")[0]


def test_a_commit_that_is_not_on_main_is_refused(tmp_path):
    r, sha, g = repo(tmp_path)
    g("tag", "goatbuilder/v0.6.0", sha["x"])
    g("tag", "goatcore/v0.6.0", sha["x"])
    assert any("not on main" in w for w in check(r, "goatcore/v0.6.0"))


def test_a_release_with_no_goatbuilder_release_is_refused(tmp_path):
    r, sha, g = repo(tmp_path)
    g("tag", "goatcore/v0.6.0", sha["c2"])
    assert any("no tag goatbuilder/v0.6.0" in w for w in check(r, "goatcore/v0.6.0"))


def test_a_goatbuilder_release_at_another_commit_is_refused(tmp_path):
    r, sha, g = repo(tmp_path)
    g("tag", "goatbuilder/v0.6.0", sha["c1"])
    g("tag", "goatcore/v0.6.0", sha["c2"])
    assert any("must be one commit" in w for w in check(r, "goatcore/v0.6.0"))


def test_stable_never_moves_backwards(tmp_path):
    r, sha, g = repo(tmp_path)
    g("branch", "stable", sha["c3"])
    g("tag", "goatbuilder/v0.6.0", sha["c2"])
    g("tag", "goatcore/v0.6.0", sha["c2"])
    assert any("only moves forward" in w for w in check(r, "goatcore/v0.6.0"))


def test_stable_moves_forward_from_an_earlier_release(tmp_path):
    r, sha, g = repo(tmp_path)
    g("branch", "stable", sha["c1"])
    g("tag", "goatbuilder/v0.7.0", sha["c3"])
    g("tag", "goatcore/v0.7.0", sha["c3"])
    assert check(r, "goatcore/v0.7.0") == []


def test_the_table_names_the_newest_release_of_each_family_in_the_commit(tmp_path):
    r, sha, g = repo(tmp_path)
    g("tag", "goat-cli-v0.5.19", sha["c1"])
    g("tag", "goatbuilder/v0.5.3", sha["c1"])
    g("tag", "goatbuilder/v0.6.0", sha["c2"])
    g("tag", "goatcore/v0.6.0", sha["c2"])
    g("tag", "goat-cli-v0.5.20", sha["c3"])  # after the release: not part of it
    text = sl.table(r, "goatcore/v0.6.0")
    assert "| goatbuilder | `goatbuilder/v0.6.0` |" in text
    assert "`goat-cli-v0.5.19`" in text and "goat-cli-v0.5.20" not in text
    assert "saltlick" in text
