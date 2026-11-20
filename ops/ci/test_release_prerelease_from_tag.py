"""The release job publishes a pre-release tag as a GitHub pre-release.

Runs the `kind` step of `.github/workflows/_release-binary.yml`'s release job
under bash for a set of tags, and checks the `prerelease` output it writes and
that the publish step passes that output on. A GitHub pre-release is never
marked Latest, so a release candidate does not replace the newest release.
"""

import os
import pathlib
import subprocess
import tempfile

import yaml

ROOT = pathlib.Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github" / "workflows" / "_release-binary.yml"


def _release_steps():
    doc = yaml.safe_load(WORKFLOW.read_text())
    return doc["jobs"]["release"]["steps"]


def _kind_step():
    steps = [s for s in _release_steps() if s.get("id") == "kind"]
    assert len(steps) == 1, "the release job has exactly one step with id kind"
    return steps[0]


def _run_kind(tag, prefix):
    step = _kind_step()
    with tempfile.TemporaryDirectory() as d:
        out = pathlib.Path(d) / "out"
        out.write_text("")
        env = {
            "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
            "GITHUB_REF_NAME": tag,
            "GITHUB_OUTPUT": str(out),
            "PREFIX": prefix,
        }
        subprocess.run(["bash", "-c", step["run"]], env=env, check=True, capture_output=True, text=True)
        lines = [l for l in out.read_text().splitlines() if l]
    assert len(lines) == 1, lines
    key, _, value = lines[0].partition("=")
    assert key == "prerelease", lines
    return value


def test_step_reads_the_callers_tag_prefix():
    assert _kind_step()["env"]["PREFIX"] == "${{ inputs.tag_prefix }}"


def test_release_candidates_are_prereleases():
    for tag, prefix in [
        ("goatbuilder/v1.0.0-rc.1", "goatbuilder/v"),
        ("goatbuilder/v1.0.0-rc.12", "goatbuilder/v"),
        ("goat-cli-v1.0.0-beta", "goat-cli-v"),
        ("goatbuilder/v1.0.0-rc.1+build.5", "goatbuilder/v"),
    ]:
        assert _run_kind(tag, prefix) == "true", tag


def test_releases_are_not_prereleases():
    for tag, prefix in [
        ("goatbuilder/v0.7.2", "goatbuilder/v"),
        ("goatbuilder/v1.0.0", "goatbuilder/v"),
        ("goat-cli-v0.4.8", "goat-cli-v"),
        # The prefix's own hyphens are not a pre-release part.
        ("goat-cli-v1.0.0+build-7", "goat-cli-v"),
    ]:
        assert _run_kind(tag, prefix) == "false", tag


def test_publish_step_passes_the_output_on():
    publish = [s for s in _release_steps() if str(s.get("uses", "")).startswith("softprops/action-gh-release@")]
    assert len(publish) == 1
    assert publish[0]["with"]["prerelease"] == "${{ steps.kind.outputs.prerelease }}"
    steps = _release_steps()
    assert steps.index(_kind_step()) < steps.index(publish[0]), "kind runs before the publish step"


if __name__ == "__main__":
    # Run as a script by coupled-fact-guards.yml; pytest collects the same tests.
    tests = [(name, fn) for name, fn in sorted(globals().items()) if name.startswith("test_") and callable(fn)]
    for name, fn in tests:
        fn()
        print(f"  ok    {name}")
    print(f"  {len(tests)} passed, 0 failed")
