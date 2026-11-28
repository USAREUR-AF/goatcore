#!/usr/bin/env python3
"""Standup phase 9 renders a fresh site's router before the portal exists, and
the role's slot-fragment refusal still stands wherever a holder could be cut off.

Runs the real task file (ops/ansible/tasks/zenoh-slot-fragment-first-render.yml)
against localhost with the router's files in a temporary directory, then the
role's own two slot-fragment tasks (read the fragment, refuse without it), taken
unchanged from roles/goat-zenoh/tasks/main.yml.
"""
import hashlib
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import unittest

import yaml

ROOT = Path(__file__).resolve().parents[1]
TASKS = ROOT / "ops/ansible/tasks/zenoh-slot-fragment-first-render.yml"
ROLE_TASKS = ROOT / "ops/ansible/roles/goat-zenoh/tasks/main.yml"
DEPLOY = ROOT / "ops/ansible/zenoh-deploy.yml"
SALT = "zagros-fixture-salt"


def anchor(k):
    return hashlib.sha256(f"{SALT}:{k}".encode()).hexdigest()[:32]


def subject(sid, cn):
    """One subject as the role's template and the portal write it."""
    return f'      {{\n        id: "{sid}",\n        cert_common_names: ["{cn}"],\n      }},\n'


def unassigned(n=4):
    return "".join(subject(f"slot-{anchor(k)}", f"{anchor(k)}.unassigned") for k in range(n))


def live_config(cns=None, region=None):
    """A live config whose slot subjects, between the portal's markers, name
    `cns`, or hold `region` as written."""
    if region is None:
        region = "".join(subject(f"slot-{anchor(k)}", cn) for k, cn in enumerate(cns))
    return (
        "{\n  access_control: {\n    subjects: [\n"
        '      { id: "founder-w", cert_common_names: ["goat-founder-trial-peer"] },\n'
        "      // BEGIN: slot subjects\n"
        "      // slot fragment sha256:" + "0" * 64 + "\n" + region + "      // END: slot subjects\n"
        "    ],\n  },\n}\n"
    )


def require_ansible_playbook(case):
    """Skip without ansible-playbook, unless GOAT_TEST_REQUIRE_ANSIBLE=1 (CI sets
    it), where its absence fails the test instead of skipping it quietly."""
    if shutil.which("ansible-playbook"):
        return
    if os.environ.get("GOAT_TEST_REQUIRE_ANSIBLE") == "1":
        case.fail("GOAT_TEST_REQUIRE_ANSIBLE=1 but ansible-playbook is not on PATH")
    case.skipTest("ansible-playbook is required")


def role_guard():
    """The role's own read-the-fragment and refuse-without-it tasks, unchanged."""
    tasks = yaml.safe_load(ROLE_TASKS.read_text())
    want = ("GHSA-rr3p | read the portal's slot fragment", "GHSA-rr3p | refuse to render slot subjects without the fragment")
    found = [t for t in tasks if t.get("name") in want]
    assert [t["name"] for t in found] == list(want), "the role's slot-fragment guard is where this test reads it"
    return found


class FirstRender(unittest.TestCase):
    def setUp(self):
        require_ansible_playbook(self)
        self._tmp = tempfile.TemporaryDirectory()
        self.dir = Path(self._tmp.name)
        self.etc = self.dir / "etc-zagros-zenoh"
        self.etc.mkdir()
        self.config = self.etc / "config.json5"
        self.fragment = self.etc / "slot-fragment.json5"
        self.portal_dir = self.dir / "opt-goat-zenoh-portal"
        self.bin = self.dir / "bin"
        self.bin.mkdir()
        self.docker("absent")

    def docker(self, state):
        """A stand-in docker on PATH: the portal container is present, absent,
        or the daemon cannot be asked."""
        body = {
            "present": 'echo "/$5"; exit 0',
            "absent": 'echo "Error response from daemon: No such container: $5" >&2; exit 1',
            "absent-object": 'echo "Error: No such object: $5" >&2; exit 1',
            "down": 'echo "Cannot connect to the Docker daemon at unix:///var/run/docker.sock." >&2; exit 1',
        }[state]
        d = self.bin / "docker"
        d.write_text("#!/bin/sh\n" + body + "\n")
        d.chmod(0o755)

    def tearDown(self):
        os.chmod(self.etc, 0o700)
        if self.config.exists() and not self.config.is_symlink():
            os.chmod(self.config, 0o600)
        self._tmp.cleanup()

    def run_play(self, ask=True, extra=(), hostvars="", more_hosts=(), path=None):
        out = self.dir / "bootstrap.out"
        play = [{
            "hosts": "routers", "gather_facts": False, "become": False,
            "vars": {
                "goat_zenoh_config_path": str(self.config),
                "goat_zenoh_slot_fragment_path": str(self.fragment),
                "zenoh_slot_fragment_bootstrap": False,
                "jz_per_vendor": False,
                "portal_data_dir": str(self.portal_dir),
            },
            "tasks": ([{"ansible.builtin.include_tasks": str(TASKS)}] if ask else []) + role_guard() + [
                {"ansible.builtin.copy": {"dest": str(out), "content": "{{ zenoh_slot_fragment_bootstrap | bool }}"}, "when": "inventory_hostname == 'router-a'"},
            ],
        }]
        if path is not None:
            play[0]["environment"] = {"PATH": path}
        pb = self.dir / "play.yml"
        pb.write_text(yaml.safe_dump(play, sort_keys=False))
        inv = self.dir / "inv.ini"
        line = " ansible_connection=local ansible_python_interpreter='{{ ansible_playbook_python }}'"
        inv.write_text("[routers]\n" + "".join(f"{h}{line}{(' ' + hostvars) if h == 'router-a' and hostvars else ''}\n" for h in ("router-a", *more_hosts)))
        env = dict(os.environ, ANSIBLE_NOCOLOR="1", ANSIBLE_RETRY_FILES_ENABLED="0", PATH=f"{self.bin}:{os.environ['PATH']}")
        r = subprocess.run(["ansible-playbook", "-i", str(inv), str(pb), *extra], capture_output=True, text=True, env=env, cwd=self.dir, timeout=180)
        return r, (out.read_text() if r.returncode == 0 else None)

    def assertFailedAt(self, r, task):
        """The play stopped at `task`: its header is followed by a fatal line,
        and the role's guard never ran."""
        after = r.stdout.split(f"| {task}]", 1)
        self.assertEqual(len(after), 2, r.stdout)
        self.assertIn("fatal: [router-a]", after[1].split("TASK [", 1)[0], r.stdout)
        self.assertNotIn("refuse to render slot subjects", r.stdout)

    def assertRefused(self, r):
        self.assertNotEqual(r.returncode, 0, r.stdout)
        self.assertIn("is missing. In per-uuid-slot-pool mode", r.stdout)

    def test_without_the_check_a_fresh_site_is_refused(self):
        # What standup phase 9 met before it asked for the check.
        r, _ = self.run_play(ask=False)
        self.assertRefused(r)

    def test_a_fresh_site_renders_without_the_fragment(self):
        r, boot = self.run_play()
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual(boot, "True")
        self.assertIn("first render", r.stdout)

    def test_a_rerun_before_the_portal_still_renders(self):
        # Phase 9 rendered every slot unassigned; the portal has not written the
        # fragment yet (phase 12 failed, or has not run).
        self.config.write_text(live_config(region=unassigned()))
        r, boot = self.run_play()
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual(boot, "True")

    def test_a_joining_sites_first_router_renders_every_slot_unassigned(self):
        # join-node phase 13 asks for the check at a joining site's core node,
        # its router_role A (the site's admin writer), where no portal runs
        # until gate row I4. A fresh router renders unassigned; the rerun finds
        # only unassigned subjects and renders them so again; one assigned slot
        # (a holder the router serves) keeps the refusal.
        r, boot = self.run_play(hostvars="router_role=A")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual(boot, "True")
        self.config.write_text(live_config(region=unassigned()))
        r, boot = self.run_play(hostvars="router_role=A")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual(boot, "True")
        self.assertIn("every slot subject in the live config is unassigned", r.stdout)
        self.config.write_text(live_config([f"{anchor(0)}.unassigned", "goat-alice-trial-peer"]))
        r, _ = self.run_play(hostvars="router_role=A")
        self.assertRefused(r)

    def test_a_router_with_one_assigned_slot_is_still_refused(self):
        cns = [f"{anchor(k)}.unassigned" for k in range(4)]
        cns[2] = anchor(2)
        self.config.write_text(live_config(cns))
        r, _ = self.run_play()
        self.assertRefused(r)
        self.assertIn("does not recognise", r.stdout)

    def test_a_later_generation_counts_as_assigned(self):
        cns = [f"{anchor(k)}.unassigned" for k in range(4)]
        cns[0] = f"{anchor(0)}.g2"
        self.config.write_text(live_config(cns))
        r, _ = self.run_play()
        self.assertRefused(r)

    def test_a_live_config_without_the_markers_is_still_refused(self):
        self.config.write_text(live_config(region=unassigned()).replace("// BEGIN: slot subjects", "// slot subjects"))
        r, _ = self.run_play()
        self.assertRefused(r)
        self.assertIn("does not recognise", r.stdout)

    def test_a_live_config_that_cannot_be_read_stops_the_play(self):
        if os.geteuid() == 0:
            self.skipTest("root reads any file")
        self.config.write_text(live_config([f"{anchor(0)}.unassigned"]))
        os.chmod(self.config, 0o000)
        r, _ = self.run_play()
        self.assertNotEqual(r.returncode, 0, r.stdout)
        self.assertFailedAt(r, "read the live router config")

    def test_a_dangling_config_link_stops_the_play(self):
        self.config.symlink_to(self.dir / "gone.json5")
        r, _ = self.run_play()
        self.assertNotEqual(r.returncode, 0, r.stdout)
        self.assertFailedAt(r, "read the live router config")

    def test_unassigned_copublish_subjects_count_as_unassigned(self):
        a = anchor(1)
        self.config.write_text(live_config(region=unassigned() + subject(f"copub-{a}-{'ab' * 8}", f"{a}.unassigned")))
        r, boot = self.run_play()
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual(boot, "True")

    def test_an_empty_region_names_no_one(self):
        self.config.write_text(live_config(region=""))
        r, boot = self.run_play()
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual(boot, "True")

    def test_shapes_the_portal_never_writes_are_refused(self):
        a = anchor(2)
        shapes = {
            "assigned co-publish": subject(f"copub-{a}-{'ab' * 8}", a),
            "unassigned name of another anchor": subject(f"slot-{a}", f"{anchor(3)}.unassigned"),
            "quoted key": f'      {{\n        id: "slot-{a}",\n        "cert_common_names": ["{a}"],\n      }},\n',
            "space before the colon": f'      {{\n        id: "slot-{a}",\n        cert_common_names : ["{a}"],\n      }},\n',
            "no certificate list": f'      {{\n        id: "slot-{a}",\n      }},\n',
            "two names": f'      {{\n        id: "slot-{a}",\n        cert_common_names: ["{a}.unassigned", "{a}"],\n      }},\n',
            "one line": f'      {{ id: "slot-{a}", cert_common_names: ["{a}.unassigned"] }},\n',
        }
        for what, extra in shapes.items():
            with self.subTest(what):
                self.config.write_text(live_config(region=unassigned() + extra))
                r, _ = self.run_play()
                self.assertRefused(r)
                self.assertIn("does not recognise", r.stdout)

    def test_a_line_break_that_ends_a_comment_hides_nothing(self):
        a = anchor(2)
        for brk in ("\r", "\u2028", "\u2029"):
            with self.subTest(repr(brk)):
                hidden = f'      // note{brk}{{ id: "slot-{a}", cert_common_names: ["{a}"] }},\n'
                self.config.write_text(live_config(region=unassigned() + hidden), newline="")
                r, _ = self.run_play()
                self.assertRefused(r)
                self.assertIn("does not recognise", r.stdout)

    def test_a_real_subject_with_a_trailing_comment_is_refused(self):
        a = anchor(2)
        region = unassigned() + f'      {{ id: "slot-{a}", cert_common_names: ["{a}"] }}, // assigned\n'
        self.config.write_text(live_config(region=region))
        r, _ = self.run_play()
        self.assertRefused(r)

    def test_one_begin_and_two_ends_are_refused(self):
        text = live_config(region=unassigned())
        self.config.write_text(text.replace("    ],\n", "      // END: slot subjects\n    ],\n", 1))
        r, _ = self.run_play()
        self.assertRefused(r)

    def test_two_regions_are_refused(self):
        text = live_config(region=unassigned())
        second = "      // BEGIN: slot subjects\n" + subject(f"slot-{anchor(0)}", anchor(0)) + "      // END: slot subjects\n"
        self.config.write_text(text.replace("    ],\n", second + "    ],\n", 1))
        r, _ = self.run_play()
        self.assertRefused(r)

    def test_a_begin_with_no_end_is_refused(self):
        self.config.write_text(live_config(region=unassigned()).replace("      // END: slot subjects\n", ""))
        r, _ = self.run_play()
        self.assertRefused(r)

    def test_no_live_config_but_a_portal_ran_here_is_refused(self):
        for what in ("portal data", "watcher loaded", "watcher reloads"):
            with self.subTest(what):
                made = {"portal data": self.portal_dir, "watcher loaded": self.etc / "slot-fragment.loaded.json5", "watcher reloads": self.etc / "slot-fragment.reloads"}[what]
                if what == "portal data":
                    made.mkdir()
                else:
                    made.write_text("x\n")
                r, _ = self.run_play()
                self.assertRefused(r)
                self.assertIn("a portal or the reload watcher has run on this host", r.stdout)
                if made.is_dir():
                    made.rmdir()
                else:
                    made.unlink()

    def test_no_live_config_but_the_portal_container_is_here_is_refused(self):
        self.docker("present")
        r, _ = self.run_play()
        self.assertRefused(r)
        self.assertIn("a portal or the reload watcher has run on this host", r.stdout)

    def test_the_older_no_such_object_wording_is_absent_too(self):
        self.docker("absent-object")
        r, boot = self.run_play()
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual(boot, "True")

    def test_no_docker_on_the_host_is_no_portal_container(self):
        empty = self.dir / "empty-path"
        empty.mkdir()
        r, boot = self.run_play(path=str(empty))
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual(boot, "True")
        self.assertIn("first render", r.stdout)

    def test_a_router_that_is_not_the_portals_host_is_refused(self):
        # An A/B pair's router B: the portal runs on A and writes only beside A's router.
        r, _ = self.run_play(hostvars="router_role=B")
        self.assertRefused(r)
        self.assertIn("is not the host the portal runs on", r.stdout)
        # The portal's home moved to B (admin_write_role=B): B is its host.
        r, boot = self.run_play(hostvars="router_role=B", extra=["-e", "admin_write_role=B"])
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual(boot, "True")
        # router_role A is the portal's host by default.
        r, boot = self.run_play(hostvars="router_role=A")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual(boot, "True")

    def test_two_routers_with_no_router_role_are_refused(self):
        r, _ = self.run_play(more_hosts=("router-b",))
        self.assertNotEqual(r.returncode, 0, r.stdout)
        self.assertIn("is not the host the portal runs on", r.stdout)

    def test_a_pattern_starting_mid_line_hides_nothing(self):
        a, b = anchor(0), anchor(1)
        # A comment that ends in a whole unassigned subject: taken out mid-line, it
        # would join the comment to the next, real subject, and hide it.
        region = ("      //" + subject(f"slot-{a}", f"{a}.unassigned")
                  + f'      {{ id: "slot-{b}", cert_common_names: ["{b}.g1"] }},\n')
        self.config.write_text(live_config(region=region))
        r, _ = self.run_play()
        self.assertRefused(r)

    def test_docker_that_cannot_answer_is_refused(self):
        self.docker("down")
        r, _ = self.run_play()
        self.assertRefused(r)
        self.assertIn("docker cannot say", r.stdout)

    def test_with_the_fragment_the_role_reads_it_and_no_bootstrap(self):
        self.fragment.write_text("// slot fragment sha256:" + "0" * 64 + "\n")
        r, boot = self.run_play()
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual(boot, "False")

    def test_an_explicit_no_wins(self):
        r, _ = self.run_play(extra=["-e", "zenoh_slot_fragment_bootstrap=false"])
        self.assertRefused(r)


class Wiring(unittest.TestCase):
    def test_the_router_play_asks_only_when_told_and_only_in_slot_pool_mode(self):
        play = yaml.safe_load(DEPLOY.read_text())[0]
        names = [t.get("name", "") for t in play["pre_tasks"]]
        inc = [t for t in play["pre_tasks"] if "zenoh-slot-fragment-first-render.yml" in str(t.get("ansible.builtin.include_tasks", ""))]
        self.assertEqual(len(inc), 1, names)
        self.assertEqual(inc[0]["when"], [
            "zenoh_slot_fragment_bootstrap_if_unassigned | default(false) | bool",
            "zenoh_identity_model == 'per-uuid-slot-pool'",
        ])

    def test_the_real_play_resolves_the_roles_paths_and_decides_before_the_role(self):
        # zenoh-deploy.yml's own include, with the role's defaults loaded as the
        # play loads them; the play ends before the role runs.
        require_ansible_playbook(self)
        real = yaml.safe_load(DEPLOY.read_text())[0]
        inc = [t for t in real["pre_tasks"] if "zenoh-slot-fragment-first-render.yml" in str(t.get("ansible.builtin.include_tasks", ""))]
        with tempfile.TemporaryDirectory() as t:
            d = Path(t)
            etc = d / "etc-zagros-zenoh"
            etc.mkdir()
            (d / "tasks").symlink_to(ROOT / "ops/ansible/tasks")
            (d / "bin").mkdir()
            (d / "bin/docker").write_text('#!/bin/sh\necho "Error: No such object: $5" >&2; exit 1\n')
            (d / "bin/docker").chmod(0o755)
            play = [{
                "hosts": "router-a", "gather_facts": False, "become": False,
                "vars": {"goat_zenoh_config_path": str(etc / "config.json5"), "portal_data_dir": str(d / "portal")},
                "pre_tasks": inc + [
                    {"ansible.builtin.copy": {"dest": str(d / "out"), "content": "{{ zenoh_slot_fragment_bootstrap | bool }} {{ goat_zenoh_slot_fragment_path }}"}},
                    {"ansible.builtin.meta": "end_play"},
                ],
                "roles": real["roles"],
            }]
            (d / "play.yml").write_text(yaml.safe_dump(play, sort_keys=False))
            (d / "inv.ini").write_text("router-a ansible_connection=local ansible_python_interpreter='{{ ansible_playbook_python }}'\n")
            env = dict(os.environ, ANSIBLE_NOCOLOR="1", ANSIBLE_RETRY_FILES_ENABLED="0", ANSIBLE_ROLES_PATH=str(ROOT / "ops/ansible/roles"), PATH=f"{d / 'bin'}:{os.environ['PATH']}")
            for args, want in [
                (["-e", "zenoh_slot_fragment_bootstrap_if_unassigned=true", "-e", "zenoh_identity_model=per-uuid-slot-pool"], "True"),
                (["-e", "zenoh_identity_model=per-uuid-slot-pool"], "False"),
                (["-e", "zenoh_slot_fragment_bootstrap_if_unassigned=true"], "False"),
            ]:
                with self.subTest(args=args):
                    r = subprocess.run(["ansible-playbook", "-i", str(d / "inv.ini"), "-e", "zenoh_stack=zagros-zenoh", *args, str(d / "play.yml")], capture_output=True, text=True, env=env, cwd=d, timeout=180)
                    self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
                    self.assertEqual((d / "out").read_text(), f"{want} {etc}/slot-fragment.json5")

    def test_the_role_writes_the_markers_and_the_unassigned_name_this_check_reads(self):
        tmpl = (ROOT / "ops/ansible/roles/goat-zenoh/templates/config.json5.j2").read_text()
        self.assertIn("\n      // BEGIN: slot subjects\n", tmpl)
        self.assertIn("\n      // END: slot subjects\n", tmpl)
        self.assertIn('      {\n        id: "slot-{{ _anchor }}",\n        cert_common_names: ["{{ _anchor }}.unassigned"],\n      },\n', tmpl)
        go = (ROOT / "ops/goat-zenoh-portal/internal/acl/slotfragment.go").read_text()
        self.assertTrue(re.search(r'SlotFragmentBegin\s*=\s*"      // BEGIN: slot subjects"', go))
        self.assertTrue(re.search(r'SlotFragmentEnd\s*=\s*"      // END: slot subjects"', go))
        self.assertTrue(re.search(r'UnassignedSuffix\s*=\s*"\.unassigned"', go))
        self.assertIn('"      {\\n        id: \\"slot-%s\\",\\n        cert_common_names: %s,\\n      },\\n"', go)
        self.assertIn('"      {\\n        id: \\"copub-%s-%s\\",\\n        cert_common_names: %s,\\n      },\\n"', go)


if __name__ == "__main__":
    unittest.main()
