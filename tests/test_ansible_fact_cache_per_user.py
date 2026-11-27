#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
#
# test_ansible_fact_cache_per_user.py: with ops/ansible/ansible.cfg, the facts a
# play gathers are cached under the running user's home directory
# (~/.cache/goat/ansible-facts), not in a directory under /tmp that every local
# user shares.
#
# A play that gathers facts runs for real against localhost with HOME set to a
# fresh directory, and the test reads where the cache file was written. Skips
# without ansible-playbook, unless GOAT_TEST_REQUIRE_ANSIBLE=1.

import os
import shutil
import subprocess
import tempfile
import unittest

REPO = os.path.normpath(os.path.join(os.path.dirname(__file__), ".."))
ANSIBLE = os.path.join(REPO, "ops", "ansible")


def require_ansible_playbook(case):
    if shutil.which("ansible-playbook"):
        return
    if os.environ.get("GOAT_TEST_REQUIRE_ANSIBLE") == "1":
        case.fail("GOAT_TEST_REQUIRE_ANSIBLE=1 but ansible-playbook is not on PATH")
    case.skipTest("ansible-playbook is required")


class FactCache(unittest.TestCase):
    def setUp(self):
        require_ansible_playbook(self)
        self.tmp = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.tmp, True)

    def test_facts_are_cached_in_the_users_home(self):
        home = os.path.join(self.tmp, "home")
        os.makedirs(home)
        play = os.path.join(self.tmp, "gather.yml")
        with open(play, "w", encoding="utf-8") as f:
            f.write("- hosts: localhost\n  connection: local\n  become: false\n"
                    "  gather_facts: true\n  gather_subset: [min]\n  tasks: []\n")
        env = {k: v for k, v in os.environ.items() if not k.startswith("ANSIBLE_CACHE_PLUGIN")}
        env.update(HOME=home, ANSIBLE_CONFIG=os.path.join(ANSIBLE, "ansible.cfg"),
                   ANSIBLE_LOCAL_TEMP=os.path.join(self.tmp, "ansible-tmp"), ANSIBLE_NOCOLOR="1",
                   ANSIBLE_RETRY_FILES_ENABLED="0")
        r = subprocess.run(["ansible-playbook", "-i", "localhost,", play], cwd=ANSIBLE, env=env,
                           capture_output=True, text=True, timeout=300)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        cache = os.path.join(home, ".cache", "goat", "ansible-facts")
        # The file name may carry a prefix before the host name.
        files = os.listdir(cache) if os.path.isdir(cache) else []
        self.assertTrue([n for n in files if n.endswith("localhost")],
                        "no fact cache for localhost under the user's home: %s" % (files,))


if __name__ == "__main__":
    unittest.main()
