#!/usr/bin/env python3
"""check-doc-signer-identity.py: every documented cosign identity is the one releases are signed with.

WHY THIS EXISTS. The documents tell a reader to check a release by hand with
`cosign verify-blob ... --certificate-identity-regexp '<regexp>'`. cosign
matches that regexp, case-sensitively, against the identity in the signing
certificate: the URL of the workflow that signed, at the tag it ran for, such as
https://github.com/<owner>/<repo>/.github/workflows/_release-binary.yml@refs/tags/goatbuilder/v0.5.3.
A regexp that does not match (a lowercase owner, a wrong workflow) makes the
documented check fail for every reader, and nothing in CI runs it.

THE IDENTITIES are read from this tree, never written here: for each workflow
that runs on a pushed tag, the workflow that signs (`cosign sign-blob`), either
itself or the local reusable workflow its job calls, at each of its tag
patterns. The repository is --release-repo OWNER/REPO when given, else
GITHUB_REPOSITORY (GitHub's own spelling of the name), or else the `origin`
remote's path.

FOR A CONSUMING REPOSITORY (an overlay assembled over goatcore that runs this
check on its own CI): GITHUB_REPOSITORY there names the consumer, not the
repository that signs the releases, so pass the release repository's name in
GitHub's case:
    python3 ops/ci/check-doc-signer-identity.py --release-repo USAREUR-AF/goatcore

WHICH VALUES ARE HELD. A quoted --certificate-identity-regexp or
--certificate-identity in docs/, README.md and .github/**/*.md is held to the
rule when it names the release repository's GitHub path
(github.com/OWNER/REPO), compared case-insensitively so a wrong-case copy is
caught, or when, read case-insensitively, it matches one of the identities. Any
other value names another signer, such as a vendor's image
(`^https://github.com/chainguard-images/.*`), and is skipped and counted. A
value holding a placeholder (`<`, `{{`, `$`) is a template and is skipped.
When values are documented but none is held, the check fails closed (exit 1)
and asks for --release-repo: a check that checked nothing does not pass.

THE RULE. A held regexp must start with `^`, must match at least one of those
identities (as cosign does: case-sensitive), and must match none of them moved
to another repository. A held exact --certificate-identity must be one of them,
with any version. Not checked: that the dots are escaped (an unescaped `.`
matches any character), and that the regexp names the signing workflow rather
than the whole repository.

Usage: check-doc-signer-identity.py [--self-test] [--release-repo OWNER/REPO]
Exit 0 ok; 1 naming each file:line, the regexp and the identities; 2 when the
repository's name or the workflows cannot be read.
"""
import argparse
import glob
import os
import re
import subprocess
import sys
import tempfile

try:
    import yaml
except ImportError:
    print("check-doc-signer-identity: PyYAML missing (pip install pyyaml)", file=sys.stderr)
    sys.exit(2)

SAMPLE_VERSION = "0.0.0"
FLAG = re.compile(r"--certificate-identity(-regexp)?[= ]+(?:'([^']*)'|\"([^\"]*)\"|(\S+))")
PLACEHOLDER = re.compile(r"<|\{\{|\$")


def repository(root, release_repo=None):
    if release_repo:
        return release_repo.strip()
    name = os.environ.get("GITHUB_REPOSITORY", "").strip()
    if name:
        return name
    try:
        url = subprocess.run(["git", "-C", root, "remote", "get-url", "origin"],
                             capture_output=True, text=True, check=True).stdout.strip()
    except (OSError, subprocess.CalledProcessError):
        return None
    m = re.search(r"[:/]([^/:]+/[^/]+?)(?:\.git)?/?$", url)
    return m.group(1) if m else None


def _load(path):
    with open(path) as f:
        text = f.read()
    return text, (yaml.safe_load(text) or {})


def identities(root, repo):
    """Every identity a release of this tree is signed with, at SAMPLE_VERSION."""
    wfdir = os.path.join(root, ".github", "workflows")
    out = []
    for path in sorted(glob.glob(os.path.join(wfdir, "*.yml")) + glob.glob(os.path.join(wfdir, "*.yaml"))):
        text, wf = _load(path)
        on = wf.get("on", wf.get(True))
        push = on.get("push") if isinstance(on, dict) else None
        tags = (push or {}).get("tags") if isinstance(push, dict) else None
        if not tags:
            continue
        signers = []
        if "cosign sign-blob" in text:
            signers.append(os.path.relpath(path, root))
        for job in (wf.get("jobs") or {}).values():
            uses = (job or {}).get("uses")
            if isinstance(uses, str) and uses.startswith("./.github/workflows/"):
                called = os.path.join(root, uses[2:].split("@")[0])
                if os.path.exists(called) and "cosign sign-blob" in _load(called)[0]:
                    signers.append(uses[2:].split("@")[0])
        for s in signers:
            for t in tags:
                out.append(f"https://github.com/{repo}/{s}@refs/tags/{str(t).replace('*', SAMPLE_VERSION)}")
    return sorted(set(out))


def documented(root):
    """(file, line, kind, value) for each documented identity flag."""
    files = [os.path.join(root, "README.md")]
    files += glob.glob(os.path.join(root, "docs", "**", "*.md"), recursive=True)
    files += glob.glob(os.path.join(root, ".github", "**", "*.md"), recursive=True)
    out = []
    for path in sorted(set(f for f in files if os.path.isfile(f))):
        with open(path, encoding="utf-8") as f:
            for n, line in enumerate(f, 1):
                for m in FLAG.finditer(line):
                    value = next(v for v in m.group(2, 3, 4) if v is not None)
                    if PLACEHOLDER.search(value):
                        continue
                    out.append((os.path.relpath(path, root), n, "regexp" if m.group(1) else "exact", value))
    return out


def names_repo(value, repo, ids):
    """True when a documented value is this repository's signer claim (see WHICH VALUES ARE HELD)."""
    plain = re.sub(r"\\([./-])", r"\1", value)
    if re.search(r"github\.com/" + re.escape(repo) + r"(?![A-Za-z0-9_-])", plain, re.IGNORECASE):
        return True
    try:
        rx = re.compile(value, re.IGNORECASE)
    except re.error:
        return False
    return any(rx.search(i) for i in ids)


def skipped(root, repo):
    """The documented values that name another signer and are not checked."""
    ids = identities(root, repo)
    return [d for d in documented(root) if not names_repo(d[3], repo, ids)]


def problems(root, repo):
    ids = identities(root, repo)
    if not ids:
        return None
    owner, name = repo.split("/", 1)
    elsewhere = [i.replace(f"/{repo}/", f"/{owner}/{name}-other/", 1) for i in ids]
    elsewhere += [i.replace(f"/{repo}/", f"/other-{owner}/{name}/", 1) for i in ids]
    out = []
    found = documented(root)
    held = [d for d in found if names_repo(d[3], repo, ids)]
    if found and not held:
        return [f"no documented identity names {repo}; every one names another signer, so nothing was checked. "
                f"If the releases are signed in another repository, pass --release-repo OWNER/REPO"]
    for path, n, kind, value in held:
        if kind == "exact":
            any_version = [re.escape(i).replace(re.escape(SAMPLE_VERSION), r"[0-9][^@/]*") for i in ids]
            if not any(re.fullmatch(p, value) for p in any_version):
                out.append(f"{path}:{n}: --certificate-identity {value!r} is no identity a release is signed with")
            continue
        if not value.startswith("^"):
            out.append(f"{path}:{n}: --certificate-identity-regexp {value!r} is not anchored: start it with "
                       f"^https://github.com/{repo}/, or cosign accepts the identity anywhere in a longer one")
            continue
        try:
            rx = re.compile(value)
        except re.error as e:
            out.append(f"{path}:{n}: --certificate-identity-regexp {value!r} is not a regexp ({e})")
            continue
        if not any(rx.search(i) for i in ids):
            out.append(f"{path}:{n}: --certificate-identity-regexp {value!r} matches no identity a release is signed "
                       f"with (cosign matches case-sensitively); the identities are: {', '.join(ids)}")
        elif any(rx.search(i) for i in elsewhere):
            out.append(f"{path}:{n}: --certificate-identity-regexp {value!r} also matches a release signed in another "
                       f"repository; anchor it on https://github.com/{repo}/")
    return out


def self_test():
    caller = ("on:\n  push:\n    tags: ['tool/v*']\njobs:\n  r:\n    uses: ./.github/workflows/_sign.yml\n")
    signer = "on: workflow_call\njobs:\n  s:\n    runs-on: x\n    steps:\n      - run: cosign sign-blob --yes x\n"
    exact = r"'^https://github\.com/Org/Repo/\.github/workflows/_sign\.yml@refs/tags/tool/v'"
    vendor = "\"^https://github.com/chainguard-images/.*\""
    # A second documented line, so a case that skips a vendor's identity still checks one of this repository's.
    also_own = " \\\n  --certificate-identity-regexp '^https://github.com/Org/Repo/'"
    # (name, documented value, expected to pass, --release-repo, GITHUB_REPOSITORY)
    cases = [
        ("the exact workflow identity, anchored", exact, True, None, None),
        ("the repository prefix in its own case", "'^https://github.com/Org/Repo/'", True, None, None),
        ("a lowercase owner", "'^https://github.com/org/repo/'", False, None, None),
        ("the wrong workflow", r"'^https://github\.com/Org/Repo/\.github/workflows/other\.yml@'", False, None, None),
        ("not anchored to the repository", "'Repo/'", False, None, None),
        ("the full prefix without a leading ^", r"'https://github\.com/Org/Repo/'", False, None, None),
        ("a placeholder is a template", "'^https://github.com/<owner>/<repo>/'", True, None, None),
        ("an exact identity at a real version", "'https://github.com/Org/Repo/.github/workflows/_sign.yml@refs/tags/tool/v1.2.3'", True, None, None),
        ("the owner alone, which also matches its other repositories", "'^https://github.com/Org/'", False, None, None),
        ("a third party's signer is not this repository's claim", vendor + also_own, True, None, None),
        ("a third party's exact identity",
         "'https://github.com/chainguard-images/images/.github/workflows/release.yaml@refs/heads/main'" + also_own,
         True, None, None),
        ("only another signer's identity: nothing is checked, so it fails closed", vendor, False, None, None),
        ("a consumer's CI without --release-repo checks nothing, so it fails closed",
         "'^https://github.com/USAREUR-AF/goatcore/'", False, None, "example-org/consumer"),
        ("--release-repo overrides GITHUB_REPOSITORY in a consumer's CI",
         "'^https://github.com/USAREUR-AF/goatcore/'", True, "USAREUR-AF/goatcore", "example-org/consumer"),
        ("a lowercase copy is refused under --release-repo",
         "'^https://github.com/usareur-af/goatcore/'", False, "USAREUR-AF/goatcore", "example-org/consumer"),
    ]
    failed = 0
    saved = os.environ.get("GITHUB_REPOSITORY")
    for name, value, ok, release_repo, env_repo in cases:
        with tempfile.TemporaryDirectory() as d:
            os.makedirs(os.path.join(d, ".github", "workflows"))
            os.makedirs(os.path.join(d, "docs"))
            open(os.path.join(d, ".github/workflows/release.yml"), "w").write(caller)
            open(os.path.join(d, ".github/workflows/_sign.yml"), "w").write(signer)
            flag = "--certificate-identity" if "@refs/" in value and not value.startswith("'^") else "--certificate-identity-regexp"
            open(os.path.join(d, "docs/install.md"), "w").write(f"cosign verify-blob \\\n  {flag} {value} \\\n")
            try:
                if env_repo is None:
                    repo = "Org/Repo"
                else:
                    os.environ["GITHUB_REPOSITORY"] = env_repo
                    repo = repository(d, release_repo)
                got = problems(d, repo)
            except Exception as e:  # a missing option is a failed case, not a crash
                got = f"{type(e).__name__}: {e}"
            finally:
                if saved is None:
                    os.environ.pop("GITHUB_REPOSITORY", None)
                else:
                    os.environ["GITHUB_REPOSITORY"] = saved
            if (got == []) != ok:
                print(f"SELF-TEST FAILED: {name}: expected {'a pass' if ok else 'a refusal'}, got {got}")
                failed += 1
    if failed:
        print(f"check-doc-signer-identity: self-test failed, {failed} of {len(cases)} cases")
        return 1
    print(f"check-doc-signer-identity: self-test ok, {len(cases)} cases")
    return 0


def main():
    parser = argparse.ArgumentParser(description="Every documented cosign identity is a release signer's.")
    parser.add_argument("--self-test", action="store_true", help="run the built-in cases and exit")
    parser.add_argument("--release-repo", metavar="OWNER/REPO",
                        help="the repository that signs the releases, in GitHub's case "
                             "(default: GITHUB_REPOSITORY, else the origin remote)")
    args = parser.parse_args()
    if args.self_test:
        return self_test()
    root = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
    repo = repository(root, args.release_repo)
    if not repo or "/" not in repo:
        print("check-doc-signer-identity: cannot read the repository's name "
              "(pass --release-repo OWNER/REPO or set GITHUB_REPOSITORY=OWNER/REPO)", file=sys.stderr)
        return 2
    found = problems(root, repo)
    if found is None:
        print("check-doc-signer-identity: no tag-triggered workflow signs a release; nothing to hold the documents to", file=sys.stderr)
        return 2
    if found:
        print("check-doc-signer-identity: a documented cosign identity is not the one releases are signed with:")
        for p in found:
            print("  " + p)
        return 1
    other = len(skipped(root, repo))
    held = len(documented(root)) - other
    tail = f", {other} names another signer and is not checked" if other == 1 else \
        f", {other} name another signer and are not checked" if other else ""
    print(f"check-doc-signer-identity: ok, {held} documented identities match {repo}'s release signers{tail}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
