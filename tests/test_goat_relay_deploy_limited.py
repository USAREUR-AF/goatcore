#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
#
# test_goat_relay_deploy_limited.py: `goatbuilder deploy goat-relay` runs
# ops/ansible/goat-relay-deploy.yml limited to the relays (`-l goat_relay_hosts`),
# so the localhost play that uploads an image archive does not run. The relays'
# archive-load step reads the URL that play sets; it must run only where that play
# ran, and leave the image to the role's registry pull otherwise.

import os
import unittest

import yaml

PLAY = os.path.join(os.path.dirname(__file__), "..", "ops", "ansible", "goat-relay-deploy.yml")


def relay_tasks():
    plays = yaml.safe_load(open(PLAY, encoding="utf-8"))
    relays = [p for p in plays if p.get("hosts") == "goat_relay_hosts"]
    assert len(relays) == 1, "one play targets goat_relay_hosts"
    p = relays[0]
    return (p.get("pre_tasks") or []) + (p.get("tasks") or []) + (p.get("post_tasks") or [])


class LimitedToTheRelays(unittest.TestCase):
    def test_the_archive_load_runs_only_where_the_archive_play_ran(self):
        loads = [t for t in relay_tasks() if "goat_relay_image_url" in str(t.get("ansible.builtin.shell") or t.get("shell") or "")]
        self.assertEqual(len(loads), 1, "one task loads the archive")
        when = loads[0].get("when")
        when = " ".join(when) if isinstance(when, list) else str(when or "")
        self.assertIn("goat_relay_image_url is defined", when)

    def test_the_archive_play_is_the_only_localhost_play(self):
        plays = yaml.safe_load(open(PLAY, encoding="utf-8"))
        self.assertEqual([p.get("name") for p in plays if p.get("hosts") == "localhost"],
                         ["push goat-relay image archive to S3 + presign for hosts"])


if __name__ == "__main__":
    unittest.main()
