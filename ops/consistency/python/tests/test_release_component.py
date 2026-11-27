#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
#
# test_release_component.py — a component that ships as a file, not an image
# (artifact_type: release, gate J23), enforced by resolve.check_release (called
# from resolve.validate). goatbuilder's pins::check_release says the same words
# (its tests: a_release_component_*).
#
# Synthetic names only. Runs under pytest (CI) and as a plain script (exit 0 =
# all pass).

import copy
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.normpath(os.path.join(HERE, "..")))

from goat_consistency import SchemaError, resolve  # noqa: E402

REL = "example-org/example-tool@v1.2.0@sha256:" + "d" * 64
IMAGE = "ghcr.io/example/app@sha256:" + "a" * 64


def release_comp():
    return {"var": "tool_release", "artifact_type": "release", "fleet": REL,
            "release": {"checksums": "SHA256SUMS",
                        "files": {"amd64": "tool-linux-amd64", "arm64": "tool-linux-arm64"},
                        "signer": "https://github.com/example-org/example-tool/.github/workflows/release.yml"}}


def refused(comp):
    """One component in a table with no goatnet: validate reads only the component."""
    try:
        resolve.validate({}, {"tool": comp})
    except SchemaError as e:
        return str(e)
    return None


def edited(fn):
    c = release_comp()
    fn(c)
    return refused(c)


def test_a_release_component_with_its_declaration_passes():
    assert refused(release_comp()) is None
    one = release_comp()
    one["release"]["files"] = {"arm64": "tool-linux-arm64"}
    assert refused(one) is None
    for fmt in ("cosign", "sigstore", "attestation"):
        named = release_comp()
        named["release"]["bundle"] = fmt
        assert refused(named) is None
    carried = release_comp()
    carried["release"]["play"] = "deploy/ansible/tool-deploy.yml"
    assert refused(carried) is None
    unset = release_comp()
    unset["fleet"] = None
    assert refused(unset) is None
    slash = release_comp()
    slash["fleet"] = REL.replace("@v1.2.0@", "@tool/v1.2.0@")
    assert refused(slash) is None, "a tag may hold a slash, as goatbuilder/v0.7.3 does"


def test_each_misdeclaration_is_refused():
    cases = [
        (lambda c: c.pop("release"), "has no `release:` mapping"),
        (lambda c: c.__setitem__("release", "SHA256SUMS"), "has no `release:` mapping"),
        (lambda c: c["release"].__setitem__("url", "x"), "unknown key 'url'"),
        (lambda c: c["release"].__setitem__("bundle", "other"), "`release.bundle:` must be one of cosign, sigstore, attestation"),
        (lambda c: c["release"].__setitem__("bundle", None), "`release.bundle:`"),
        (lambda c: c["release"].__setitem__("bundle", ["cosign"]), "`release.bundle:`"),
        (lambda c: c["release"].__setitem__("play", "../play.yml"), "`release.play:` is a play's path"),
        (lambda c: c["release"].__setitem__("play", "deploy/play.sh"), "`release.play:` is a play's path"),
        (lambda c: c["release"].__setitem__("play", None), "`release.play:` is a play's path"),
        (lambda c: (c["release"].__setitem__("play", "deploy/play.yml"), c.__setitem__("deploy_playbook", "tool-deploy.yml")), "its play is in one place"),
        (lambda c: (c["release"].__setitem__("play", "deploy/play.yml"), c.__setitem__("deploy_playbook", None)), "its play is in one place"),
        (lambda c: c["release"].pop("checksums"), "`release.checksums:`"),
        (lambda c: c["release"].__setitem__("checksums", "../SHA256SUMS"), "`release.checksums:`"),
        (lambda c: c["release"].__setitem__("checksums", "tool.attestations.json"), "`release.checksums:`"),
        (lambda c: c["release"]["files"].__setitem__("amd64", "tool-linux-arm64.attestations.json"), "`release.files.amd64:`"),
        (lambda c: c["release"].__setitem__("files", {}), "`release.files:` must map"),
        (lambda c: c["release"]["files"].__setitem__("riscv64", "tool"), "architecture 'riscv64'"),
        (lambda c: c["release"]["files"].__setitem__("amd64", "bin/tool"), "`release.files.amd64:`"),
        (lambda c: c["release"].__setitem__("signer", " "), "`release.signer:`"),
        (lambda c: c["release"].__setitem__("signer", "https://github.com/example-org/example-tool/.github/workflows/release.yml@refs/tags/v1.2.0"), "with no ref"),
        (lambda c: c["release"].__setitem__("signer", "example-org/example-tool"), "`release.signer:` is the workflow"),
        (lambda c: c.__setitem__("fleet", "example-org/example-tool@tool//v1@sha256:" + "d" * 64), "is not a release pin"),
        (lambda c: c.__setitem__("fleet", REL + "\n"), "is not a release pin"),
        (lambda c: c["release"].__setitem__("signer", c["release"]["signer"] + "\n"), "`release.signer:`"),
        (lambda c: c.__setitem__("fleet", IMAGE), "is not a release pin"),
        (lambda c: c.__setitem__("fleet", "example-org/example-tool@sha256:" + "d" * 64), "is not a release pin"),
        # One line only: `$` alone would match before a trailing newline.
        (lambda c: c["release"].__setitem__("signer", "https://github.com/example-org/example-tool/.github/workflows/release.yml\n"), "`release.signer:` is the workflow"),
        (lambda c: c.__setitem__("fleet", REL + "\n"), "is not a release pin"),
        # The names the release-file role accepts on the host, and no other.
        (lambda c: c["release"]["files"].__setitem__("amd64", "tool linux amd64"), "`release.files.amd64:`"),
        (lambda c: c["release"].__setitem__("checksums", "SHA256SUMS;x"), "`release.checksums:`"),
    ]
    for fn, want in cases:
        msg = edited(fn)
        assert msg and want in msg, (want, msg)


def test_a_release_deviation_carries_a_release_pin_too():
    c = release_comp()
    devs = {"zagros": {"value": "example-org/example-tool@v1.3.0-rc.1", "reason": "r"}}
    try:
        resolve.check_release("tool", c, c["fleet"], devs)
        msg = None
    except SchemaError as e:
        msg = str(e)
    assert msg and "deviation 'zagros'" in msg and "is not a release pin" in msg, msg
    resolve.check_release("tool", c, c["fleet"], {"zagros": {"value": REL.replace("v1.2.0", "v1.3.0"), "reason": "r"}})
    resolve.check_release("tool", c, c["fleet"], {"zagros": {"absent": True, "reason": "r"}})


def test_a_release_pins_repository_must_be_its_signers_repository():
    msg = edited(lambda c: c.__setitem__("fleet", REL.replace("example-org/example-tool@", "other-org/example-tool@")))
    assert msg and ("the pin's repository 'other-org/example-tool' is not the repository of "
                    "`release.signer:` ('example-org/example-tool')") in msg, msg
    msg = edited(lambda c: c.__setitem__("fleet", REL.replace("/example-tool@", "/example-tool-fork@")))
    assert msg and "a release pin's repository must be its signer's repository" in msg, msg
    c = release_comp()
    try:
        resolve.check_release("tool", c, c["fleet"], {"zagros": {"value": REL.replace("example-org/", "other-org/"), "reason": "r"}})
        msg = None
    except SchemaError as e:
        msg = str(e)
    assert msg and "deviation 'zagros'" in msg and "must be its signer's repository" in msg, msg
    assert edited(lambda c: c.__setitem__("fleet", REL.replace("example-org/example-tool@", "Example-Org/Example-Tool@"))) is None, \
        "GitHub's names are not case-sensitive"


def test_only_a_release_component_names_a_release():
    c = copy.deepcopy(release_comp())
    c["artifact_type"] = "image"
    c["fleet"] = IMAGE
    msg = refused(c)
    assert msg and "declares `release:` but its artifact_type is 'image'" in msg, msg


if __name__ == "__main__":
    failed = 0
    for name, fn in sorted(globals().items()):
        if name.startswith("test_") and callable(fn):
            try:
                fn()
                print("ok   %s" % name)
            except AssertionError as e:
                failed += 1
                print("FAIL %s: %s" % (name, e))
    sys.exit(1 if failed else 0)
