"""The onboard-runner play puts goat on a host that has none, from the pinned
release, verified, before the step that requires it. A founder stand-up
reaches the runner on a host nothing has installed goat on."""
import pathlib
import unittest

import yaml

ROLE = pathlib.Path(__file__).resolve().parents[1] / "ops/ansible/roles/onboard-runner"


def names(tasks):
    return [t.get("name", "") for t in tasks]


class InstallsGoat(unittest.TestCase):
    def setUp(self):
        self.tasks = yaml.safe_load((ROLE / "tasks/main.yml").read_text())
        self.defaults = yaml.safe_load((ROLE / "defaults/main.yml").read_text())

    def test_the_release_install_comes_before_the_requirement(self):
        n = names(self.tasks)
        install = n.index("Install the pinned goat-cli release on a host with no usable goat")
        require = next(i for i, x in enumerate(n) if x.startswith("Require goat >="))
        probe = n.index("Probe the host goat binary version")
        self.assertLess(install, probe)
        self.assertLess(probe, require)

    def test_nothing_is_installed_unverified(self):
        block = next(t for t in self.tasks if t.get("name", "").startswith("Install the pinned goat-cli release"))
        script = block["block"][0]["ansible.builtin.shell"]
        self.assertIn(".sha256", script)
        self.assertLess(script.index("sha256sum"), script.index("install -o root"))
        self.assertIn('[ "$want" = "$got" ]', script)

    def test_a_host_with_a_usable_goat_or_a_served_copy_is_left_alone(self):
        block = next(t for t in self.tasks if t.get("name", "").startswith("Install the pinned goat-cli release"))
        when = " ".join(str(w) for w in block["when"])
        self.assertIn("onboard_runner_served_bin.stat.exists", when)
        self.assertIn("onboard_runner_min_goat_version", when)

    def test_one_pin(self):
        self.assertIn("goat_get_release_tag", self.defaults["onboard_runner_goat_release_tag"])


if __name__ == "__main__":
    unittest.main()
