#!/usr/bin/env python3
"""Every Ubuntu image CI builds with apt takes the runner's mirror (UBUNTU_MIRROR).

.github/actions/apt-failover probes the runner's own Ubuntu mirror and, when it answers,
exports it as UBUNTU_MIRROR. A docker build does not use the runner's apt sources: an
ubuntu image's apt goes to archive.ubuntu.com and security.ubuntu.com, which a runner can
reach badly while its own mirror is fine (19 Nov 2026: peering-forwarder and
peering-host-firewall timed out on every pull request in their fixture builds, while the
action reported the mirror healthy). So every such build passes the mirror as a build
argument, and its Dockerfile points apt at it when it is set. Unset (a local run), the
image's own sources stay.

Checked here, over the tracked tree:
  - a shell script that runs `docker build` on an inline Dockerfile from an ubuntu image
    with apt-get passes `--build-arg UBUNTU_MIRROR=` and declares `ARG UBUNTU_MIRROR`;
  - a Dockerfile from an ubuntu image that runs apt-get declares `ARG UBUNTU_MIRROR` and
    rewrites the sources with it, unless EXEMPT names it with the reason;
  - a script that builds one of those Dockerfiles passes `--build-arg UBUNTU_MIRROR=`;
  - the action exports UBUNTU_MIRROR on the path where the mirror answered.

Run: python3 -m pytest ops/ci/test_docker_apt_builds_take_ubuntu_mirror.py
"""
import re
import subprocess
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]

# Ubuntu Dockerfiles with apt that CI does not build on the normal path, and why.
EXEMPT = {
    "ops/bootstrap/first-boot/integration-test/Dockerfile":
        "run by hand (its run-test.sh); no workflow builds it",
    "ops/federation-e2e/host/Dockerfile":
        "the federation test environment, run locally; in CI only the apt-failover selftest's "
        "forced job builds it, where the archive is black-holed and our cache serves it",
    "ops/netbird/Dockerfile":
        "a published image (netbird-mgmt.yml), not a CI fixture: a release keeps the image's "
        "own sources, so what it installs never depends on the runner that built it",
}

UBUNTU_FROM = re.compile(r"^FROM\s+(?:--platform=\S+\s+)?ubuntu:", re.M)
# An inline Dockerfile whose base is a variable that defaults to an ubuntu image.
UBUNTU_DEFAULT = re.compile(r'^([A-Z0-9_]+)="\$\{[A-Z0-9_]+:-ubuntu:[^} ]*\}"', re.M)


def builds_from_ubuntu(t):
    if UBUNTU_FROM.search(t):
        return True
    return any(re.search(r"^FROM\s+\$\{?%s\}?\s*$" % m.group(1), t, re.M) for m in UBUNTU_DEFAULT.finditer(t))


def tracked(*patterns):
    out = subprocess.run(["git", "-C", str(REPO), "ls-files", *patterns],
                         capture_output=True, text=True, check=True).stdout
    return [p for p in out.splitlines() if p]


def text(path):
    return (REPO / path).read_text(encoding="utf-8", errors="replace")


def ubuntu_dockerfiles():
    out = []
    for p in tracked():
        name = p.rsplit("/", 1)[-1]
        if not (name.startswith("Dockerfile") or name.endswith(".Dockerfile")):
            continue
        t = text(p)
        if UBUNTU_FROM.search(t) and "apt-get" in t:
            out.append(p)
    return out


class DockerAptBuildsTakeTheMirror(unittest.TestCase):
    def test_inline_ubuntu_builds_pass_and_declare_the_mirror(self):
        found = []
        for p in tracked("*.sh"):
            t = text(p)
            if "docker build" not in t or "apt-get" not in t:
                continue
            if not builds_from_ubuntu(t):
                continue
            found.append(p)
            with self.subTest(script=p):
                self.assertTrue("--build-arg UBUNTU_MIRROR=" in t,
                              f"{p} builds an ubuntu image with apt but does not pass UBUNTU_MIRROR")
                self.assertTrue("ARG UBUNTU_MIRROR" in t,
                              f"{p}'s inline Dockerfile does not declare ARG UBUNTU_MIRROR")
        # The two builds this exists for are among them.
        for p in ("ops/ci/test-peering-forwarder-role.sh", "ops/ci/test-host-firewall-role.sh"):
            self.assertIn(p, found)

    def test_ubuntu_dockerfiles_rewrite_their_sources_with_the_mirror(self):
        for p in ubuntu_dockerfiles():
            if p in EXEMPT:
                continue
            t = text(p)
            with self.subTest(dockerfile=p):
                self.assertTrue("ARG UBUNTU_MIRROR" in t, f"{p} does not declare ARG UBUNTU_MIRROR")
                self.assertTrue("ubuntu.sources" in t, f"{p} does not point its apt sources at UBUNTU_MIRROR")

    def test_the_exemptions_are_still_ubuntu_apt_dockerfiles(self):
        files = set(ubuntu_dockerfiles())
        for p in EXEMPT:
            self.assertIn(p, files, f"{p} is exempt but no longer an ubuntu Dockerfile with apt: drop it")

    def test_scripts_that_build_those_dockerfiles_pass_the_mirror(self):
        names = {p for p in ubuntu_dockerfiles() if p not in EXEMPT}
        self.assertIn("ops/e2e/host/Dockerfile", names)
        # ops/e2e/hermetic.sh builds ops/e2e/host/<Dockerfile> for the deployment e2e.
        t = text("ops/e2e/hermetic.sh")
        line = next(l for l in t.splitlines() if "docker build" in l and "HOST_DOCKERFILE" in l)
        self.assertTrue("--build-arg UBUNTU_MIRROR=" in line, "ops/e2e/hermetic.sh builds the host image without UBUNTU_MIRROR")

    def test_the_action_exports_the_mirror_it_proved(self):
        t = text(".github/actions/apt-failover/action.yml")
        i = t.index('answered in $(( $(date +%s) - t0 ))s; apt and docker unchanged')
        branch = t[t.rindex("if probe; then", 0, i):t.index("exit 0", i)]
        self.assertTrue('UBUNTU_MIRROR=' in branch, 'the action does not export UBUNTU_MIRROR where the mirror answered')
        self.assertTrue('>> "$GITHUB_ENV"' in branch, 'the export does not go to GITHUB_ENV')


if __name__ == "__main__":
    unittest.main()
