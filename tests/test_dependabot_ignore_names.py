"""Every docker ignore rule in .github/dependabot.yml names an image the way Dependabot does.

Dependabot names a docker dependency by its image path without the registry host:
`FROM ghcr.io/pgedge/pgedge-postgres:16-...` is `pgedge/pgedge-postgres`, and
`FROM cgr.dev/chainguard/go:latest` is `chainguard/go` (its pull request bodies
say "Updates `chainguard/go` ..."). An ignore rule whose dependency-name carries the
host matches nothing, and Dependabot proposes exactly what the rule was written to
stop. So each docker ignore rule must name an image that a FROM line in one of
that entry's directories pulls, written without the host.

Also: the Postgres image's major-version ignore is in place, since a Postgres
major upgrade is a planned migration (pg_upgrade), never a dependency bump.

Run: python3 -m pytest -q tests/test_dependabot_ignore_names.py
"""

import pathlib
import re
import unittest

import yaml

ROOT = pathlib.Path(__file__).resolve().parents[1]
CONFIG = ROOT / ".github" / "dependabot.yml"
FROM = re.compile(r"^\s*FROM\s+(?:--platform=\S+\s+)?(\S+)", re.IGNORECASE | re.MULTILINE)


def dependabot_name(ref: str) -> str:
    """The name Dependabot gives the image of a FROM reference."""
    image = ref.split("@", 1)[0]
    last = image.rsplit("/", 1)[-1]
    if ":" in last:
        image = image[: len(image) - len(last)] + last.split(":", 1)[0]
    parts = image.split("/")
    if len(parts) > 1 and ("." in parts[0] or ":" in parts[0] or parts[0] == "localhost"):
        parts = parts[1:]
    if len(parts) == 2 and parts[0] == "library":
        parts = parts[1:]
    return "/".join(parts)


def docker_entries():
    cfg = yaml.safe_load(CONFIG.read_text())
    return [u for u in cfg["updates"] if u.get("package-ecosystem") == "docker"]


def images_in(entry) -> set:
    dirs = entry.get("directories") or [entry.get("directory", "/")]
    names = set()
    for d in dirs:
        for df in (ROOT / d.lstrip("/")).glob("*Dockerfile*"):
            for ref in FROM.findall(df.read_text()):
                if "$" not in ref:
                    names.add(dependabot_name(ref))
    return names


class IgnoreNames(unittest.TestCase):
    def test_name_rule(self):
        self.assertEqual(dependabot_name("ghcr.io/pgedge/pgedge-postgres:16-spock5-minimal@sha256:ab"),
                         "pgedge/pgedge-postgres")
        self.assertEqual(dependabot_name("cgr.dev/chainguard/go:latest@sha256:ab"), "chainguard/go")
        self.assertEqual(dependabot_name("golang:1.26.11-bookworm@sha256:ab"), "golang")
        self.assertEqual(dependabot_name("docker.io/library/alpine:3"), "alpine")

    def test_every_docker_ignore_names_a_pulled_image_without_its_host(self):
        for entry in docker_entries():
            pulled = images_in(entry)
            for rule in entry.get("ignore") or []:
                name = rule["dependency-name"]
                self.assertIn(name, pulled,
                              f"ignore dependency-name {name!r} is not how Dependabot names any image this "
                              f"entry pulls (it drops the registry host); pulled: {sorted(pulled)}")


class PostgresMajor(unittest.TestCase):
    def test_postgres_major_versions_ignored(self):
        rules = [r for e in docker_entries() for r in (e.get("ignore") or [])
                 if r["dependency-name"] == "pgedge/pgedge-postgres"]
        self.assertTrue(rules, "the pgedge/pgedge-postgres image needs an ignore rule")
        r = rules[0]
        self.assertIn("version-update:semver-major", r.get("update-types", []),
                      "semver-major updates of the Postgres image must be ignored")
        self.assertIn(">=17", r.get("versions", []),
                      "the range must stay, for a tag Dependabot does not read as semver")


if __name__ == "__main__":
    unittest.main()
