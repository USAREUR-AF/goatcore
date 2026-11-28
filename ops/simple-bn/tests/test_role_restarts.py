# SPDX-License-Identifier: Apache-2.0
"""The role restarts every process that runs code it installs.

bn-monitor and the receiver's monitor (bn-monitor-receiver, monitor.receiver: true) both run
bn_monitor.py. A deploy that changes the monitor code must restart both, or the receiver keeps
serving the previous API. Reads the role's YAML; needs PyYAML (CI installs it, boundary-and-door.yml).
"""
import os
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.abspath(os.path.join(HERE, "..", "..", ".."))
ROLE = os.path.join(REPO, "ops", "ansible", "roles", "simple-bn")

try:
    import yaml
except ImportError:
    yaml = None


def _load(*parts):
    with open(os.path.join(ROLE, *parts)) as f:
        return yaml.safe_load(f)


def _notified(task):
    n = task.get("notify") or []
    return [n] if isinstance(n, str) else list(n)


@unittest.skipIf(yaml is None, "needs PyYAML")
class MonitorCodeRestartsBothMonitors(unittest.TestCase):
    def setUp(self):
        self.tasks = _load("tasks", "main.yml")
        self.handlers = {h["name"]: h for h in _load("handlers", "main.yml")}

    def code_task(self):
        for t in self.tasks:
            loop = t.get("loop") or []
            if isinstance(loop, list) and "bn_monitor.py" in loop:
                return t
        self.fail("no task installs bn_monitor.py")

    def test_the_monitor_code_task_notifies_a_restart_of_each_monitor(self):
        notified = _notified(self.code_task())
        restarted = set()
        for name in notified:
            self.assertIn(name, self.handlers, f"notify names a handler that does not exist: {name}")
            h = self.handlers[name]["ansible.builtin.systemd"]
            names = self.handlers[name].get("loop") or [h.get("name")]
            restarted.update(names)
            self.assertEqual(h.get("state"), "restarted")
        self.assertTrue({"bn-monitor", "bn-monitor-receiver"} <= restarted, restarted)

    def test_the_receiver_restart_runs_only_where_the_receiver_monitor_is_declared(self):
        h = self.handlers["restart bn-monitor-receiver"]
        self.assertIn("simple_bn.monitor.receiver", str(h.get("when", "")))

    def test_handlers_are_flushed_after_the_receiver_unit_exists(self):
        names = [t.get("name", "") for t in self.tasks]
        unit = names.index("simple-bn | the receiver's monitor running")
        flushes = [i for i, t in enumerate(self.tasks) if t.get("ansible.builtin.meta") == "flush_handlers"]
        self.assertTrue(flushes, "no flush_handlers")
        self.assertGreater(flushes[0], unit, "handlers would run before the receiver unit is installed")


if __name__ == "__main__":
    unittest.main()
