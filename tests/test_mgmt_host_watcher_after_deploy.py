#!/usr/bin/env python3
"""mgmt-host.yml starts the stack's watcher only after the stack is up, and pauses it meanwhile.

A play's `roles:` run before its `tasks:`. mgmt-host-compose-watch was in the
deploy play's roles, so on a re-deploy the watcher was enabled and started
before the play wrote the new docker-compose.yml and ran `up -d`. A joiner's
re-initialisation then had the watcher start the OLD postgres-a, from the old
compose file still on disk, on the emptied volume: initdb ran without the
founder's encoding.

Now:
- the watcher role runs in the play's post_tasks, after the stack is up, and
  `--tags compose-watch` still selects it;
- the play's tasks are one block. Its first task writes the watcher's pause
  file with the reason mgmt-host-deploy, unless a pause is already there; its
  last task removes that pause and only that one;
- the block's rescue says, loudly, that the watcher is still paused and how
  to clear it, then fails the play as before.

Reads the play and runs the pause and clear commands against a temporary file.
"""
from pathlib import Path
import subprocess
import tempfile
import unittest

import yaml

ROOT = Path(__file__).resolve().parents[1]
PLAY = ROOT / "ops/ansible/mgmt-host.yml"
PAUSE = "/run/mgmt-host-compose-watch.paused"
ROLE = "mgmt-host-compose-watch"


def deploy_play():
    plays = yaml.safe_load(PLAY.read_text())
    return next(p for p in plays if p.get("name") == "Deploy management-plane compose stack")


def role_name(r):
    return r if isinstance(r, str) else r.get("role") or r.get("name")


def shell(t):
    return str(t.get("ansible.builtin.shell", t.get("shell", "")))


class TheWatcherStartsAfterTheStack(unittest.TestCase):
    def test_the_watcher_role_is_not_in_roles(self):
        names = [role_name(r) for r in deploy_play().get("roles", [])]
        self.assertNotIn(ROLE, names, "a play's roles run before its tasks")

    def test_post_tasks_import_the_watcher_role_with_its_tag(self):
        post = deploy_play().get("post_tasks", [])
        imp = [t for t in post if (t.get("ansible.builtin.import_role") or {}).get("name") == ROLE]
        self.assertEqual(len(imp), 1, post)
        self.assertIn("compose-watch", imp[0].get("tags", []))


class TheDeployPausesTheWatcher(unittest.TestCase):
    def block(self):
        tasks = deploy_play()["tasks"]
        self.assertEqual(len(tasks), 1, "the play's tasks are one block")
        self.assertIn("block", tasks[0])
        return tasks[0]

    def test_the_first_task_pauses_and_the_last_clears_after_the_stack_is_up(self):
        b = self.block()["block"]
        names = [t.get("name", "") for t in b]
        self.assertIn(PAUSE, shell(b[0]))
        self.assertIn("mgmt-host-deploy", shell(b[0]))
        self.assertIn(PAUSE, shell(b[-1]))
        up = names.index("Bring up stack")
        self.assertLess(0, up)
        self.assertLess(up, len(b) - 1)

    def test_the_rescue_says_the_watcher_is_paused_and_how_to_clear_it(self):
        rescue = self.block().get("rescue", [])
        text = yaml.safe_dump(rescue)
        self.assertIn(PAUSE, text)
        self.assertIn("rm", text)
        self.assertTrue(any("ansible.builtin.fail" in t or "fail" in t for t in rescue), "the play still fails")

    def run_cmd(self, cmd, content):
        with tempfile.TemporaryDirectory() as d:
            f = Path(d) / "paused"
            if content is not None:
                f.write_text(content)
            out = subprocess.run(["bash", "-c", cmd.replace(PAUSE, str(f))], capture_output=True, text=True)
            self.assertEqual(out.returncode, 0, out.stderr)
            return (f.read_text() if f.exists() else None), out.stdout

    def test_the_pause_is_written_only_when_none_is_there(self):
        cmd = shell(self.block()["block"][0])
        got, _ = self.run_cmd(cmd, None)
        self.assertEqual((got or "").strip(), "mgmt-host-deploy")
        got, _ = self.run_cmd(cmd, "postgres-reinit\n")
        self.assertEqual(got, "postgres-reinit\n", "another procedure's pause is kept as it is")

    def test_the_clear_removes_only_the_deploys_own_pause(self):
        cmd = shell(self.block()["block"][-1])
        got, out = self.run_cmd(cmd, "mgmt-host-deploy\n")
        self.assertIsNone(got)
        got, _ = self.run_cmd(cmd, "postgres-reinit\n")
        self.assertEqual(got, "postgres-reinit\n")
        got, _ = self.run_cmd(cmd, None)
        self.assertIsNone(got)


if __name__ == "__main__":
    unittest.main()
