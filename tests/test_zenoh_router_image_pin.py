#!/usr/bin/env python3
"""goat-zenoh's default router image is pinned the way the pin table pins:
`<registry>/<repo>:<tag>@sha256:<multi-arch index digest>`, so the tag names the
build a digest came from and a digest nobody published cannot hide behind it.
The federation harnesses run the same digest.

The default once named zenoh-pqc@sha256:372de39d..., which GHCR does not hold,
so every site without a fleet pin for zenoh_router_image failed its pull.
Whether a digest is in the registry needs the network; the PR records that
check. This test holds the shape and the lockstep offline."""
import re
import sys
import unittest
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "ops/consistency/python"))
from goat_consistency import resolve  # noqa: E402

DEFAULTS = ROOT / "ops/ansible/roles/goat-zenoh/defaults/main.yml"
PINNED = re.compile(r"^ghcr\.io/\{\{ ghcr_org \}\}/zenoh-pqc:(sha-[0-9a-f]{7,40})@sha256:([0-9a-f]{64})$")
UNPUBLISHED = {"372de39d2e8b64fe7157194173f1b7ac6eac2700ff04c49cce2ce3c284a6fdb2"}
LOCKSTEP = ["ops/federation-e2e/live.sh", "ops/federation-e2e/run.sh", "ops/federation-smoke/README.md"]


def default_image():
    return yaml.safe_load(DEFAULTS.read_text())["zenoh_router_image"]


class RouterImagePin(unittest.TestCase):
    def test_default_is_tag_at_digest(self):
        img = default_image()
        self.assertRegex(img, PINNED)
        self.assertRegex(img, resolve.DIGEST_RE, "the pin table's digest rule")

    def test_default_is_not_a_digest_the_registry_lacks(self):
        self.assertNotIn(PINNED.match(default_image()).group(2), UNPUBLISHED)

    def test_the_harnesses_run_the_same_build(self):
        tag, digest = PINNED.match(default_image()).groups()
        for rel in LOCKSTEP:
            text = (ROOT / rel).read_text()
            refs = re.findall(r"zenoh-pqc[:@][^\s'\"]+", text)
            self.assertTrue(refs, rel)
            for ref in refs:
                self.assertEqual(ref, f"zenoh-pqc:{tag}@sha256:{digest}", rel)


if __name__ == "__main__":
    unittest.main()
