"""The plan's host firewall hash equals the deploy's, for the shared fixture.

`goatbuilder federate plan` builds the renderer input itself (crates/goatbuilder/
src/federate/hostfw.rs), and a Rust test pins what it builds for the fixture to
ops/federation/standup/fixtures/host-firewall/expected/<host>.json. This test
has Ansible build the deploy's input for the same fixture, from the role's and
the plays' OWN tasks and variables, copied verbatim out of their YAML:

- the transport play's peering entry (`goat_host_firewall_peering` of peering-wg.yml
  or, for an ipsec-fips agreement, peering-edge.yml) and the core play's SPIRE
  variables (peering-republisher.yml);
- goat-peering-wg's tasks that read the counterparty package and derive the peer address;
- the role's "every peering of this host" and "assemble the input" tasks;
- the defaults of every role the play has in scope, and apply's rendered vars
  through the real tasks/peering-applied-vars.yml.

Each fixture variant (wireguard, ipsec, forwarder) runs once.

The play passes its variables to the role as include_role vars; here they are a
set_fact in the role. Both outrank inventory, host_vars and group_vars, and
apply's include_vars never sets a goat_host_firewall_* name, so the values are
the same. Only the live reads are stubbed (the stored peerings come from the fixture's
probe answer, the same entries the plan reads; no bridges, uplinks or session).
It then renders both inputs with the real renderer, in this run, and requires
the same sha256. A changed expression in the role or the plays that the plan
does not follow makes the hashes differ here.

Needs ansible-playbook (the CI python job installs ansible-core); skipped without it.
"""
import base64
import copy
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

import yaml

ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "ops/federation/standup/fixtures/host-firewall"
VARIANTS = ("wireguard", "ipsec", "forwarder")
ROLES = ROOT / "ops/ansible/roles"
RENDERER = ROLES / "goat-host-firewall/files/goat-host-firewall-render.py"
LIVE_HOST = ("bridge_interfaces", "ephemeral_ports", "uplink_interfaces")
LIVE_TOP = ("must_reach", "listening_ss", "listening")


def load(path):
    return yaml.safe_load(Path(path).read_text())


def task(tasks, name):
    for t in tasks:
        if t.get("name") == name:
            return copy.deepcopy(t)
    raise AssertionError(f"no task named {name!r}")


def baseline_vars(play, profile):
    for t in load(ROOT / "ops/ansible" / play)[0]["post_tasks"]:
        if (t.get("vars") or {}).get("goat_host_firewall_profile") == profile:
            return copy.deepcopy(t["vars"])
    raise AssertionError(f"{play} has no {profile} baseline task")


def defaults(*roles):
    merged = {}
    for r in roles:
        d = load(ROLES / r / "defaults/main.yml") or {}
        overlap = set(d) & set(merged)
        assert not overlap, f"{r} and another role in scope both default {sorted(overlap)}"
        merged.update(d)
    return merged


def render(doc):
    r = subprocess.run(["python3", str(RENDERER), "--input", "-", "--format", "json"],
                       input=json.dumps(doc), capture_output=True, text=True)
    if r.returncode != 0:
        raise AssertionError(f"renderer refused ({r.returncode}): {r.stderr.strip()}")
    return json.loads(r.stdout)["sha256"]


@unittest.skipUnless(shutil.which("ansible-playbook"), "ansible-playbook is not installed")
class PlanEqualsDeploy(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)

    def prepare(self, variant):
        self.fixture = FIXTURES / variant
        self.root = Path(tempfile.mkdtemp(dir=self.temp.name))
        self.play_dir = self.root / "ansible"
        shutil.copytree(self.fixture / "repo/ops/ansible", self.play_dir)
        (self.play_dir / "tasks").mkdir()
        shutil.copy(ROOT / "ops/ansible/tasks/peering-applied-vars.yml", self.play_dir / "tasks")
        self.work = self.root / "work"
        shutil.copytree(self.fixture / "work", self.work)
        self.out = self.root / "out"
        self.out.mkdir()
        self.transport = json.loads((self.work / "agreement.json").read_text())["transport"]["kind"]

    def role(self, name, role_defaults, tasks):
        d = self.play_dir / "roles" / name
        (d / "defaults").mkdir(parents=True)
        (d / "tasks").mkdir()
        (d / "defaults/main.yml").write_text(yaml.safe_dump(role_defaults, sort_keys=True))
        (d / "tasks/main.yml").write_text(yaml.safe_dump(tasks, sort_keys=False))

    def stubs(self, host):
        probe = self.work / "preflight/probe" / f"{host}.json"
        stored = json.loads(probe.read_text())["host_firewall_peerings"] if probe.is_file() else []
        return {"name": "stubs for the live reads", "ansible.builtin.set_fact": {
            "_hf_state": {"bridges": [], "ephemeral": []},
            "_hf_session": {"skipped": True, "stdout": ""},
            "_hf_ss": {"stdout": ""},
            "_hf_stored": {"results": [{"content": base64.b64encode(json.dumps(p).encode()).decode()}
                                       for p in stored]},
        }}

    def build(self):
        fw = load(ROLES / "goat-host-firewall/tasks/main.yml")
        dump = {"name": "hand the input over", "ansible.builtin.copy": {
            "content": "{{ _hf_input | to_json }}", "dest": "{{ out }}/{{ inventory_hostname }}.json"},
            "delegate_to": "localhost"}
        if self.transport == "wireguard":
            wg = load(ROLES / "goat-peering-wg/tasks/main.yml")
            edge_roles = ("goat-host-firewall", "goat-peering-wg")
            edge_pre = [task(wg, "peering-wg | read the counterparty's package"),
                        task(wg, "peering-wg | compose what the offer and the agreement say")]
            edge_vars = baseline_vars("peering-wg.yml", "edge")
        else:
            edge_roles = ("goat-host-firewall", "goat-peering-edge")
            edge_pre = []
            edge_vars = baseline_vars("peering-edge.yml", "edge")
        core_vars = baseline_vars("peering-republisher.yml", "core")
        self.role("plan_eval_edge", defaults(*edge_roles), edge_pre + [
            {"name": "the play's variables for the role", "ansible.builtin.set_fact": edge_vars},
            self.stubs("edge-a"),
            task(fw, "host-firewall | every peering of this host, this run's in place of its stored copy"),
            task(fw, "host-firewall | assemble the input"),
            dump,
        ])
        self.role("plan_eval_core", defaults("goat-host-firewall", "goat-fed-republisher"), [
            {"name": "the play's variables for the role", "ansible.builtin.set_fact": core_vars},
            self.stubs("core-a"),
            task(fw, "host-firewall | assemble the input"),
            dump,
        ])
        # peering-wg.yml and peering-republisher.yml load apply's rendered
        # values first; peering-edge.yml (ipsec-fips) does not.
        applied = {"ansible.builtin.include_tasks": "tasks/peering-applied-vars.yml"}
        playbook = [
            {"hosts": "velmara_edge", "gather_facts": False, "tasks":
                ([applied] if self.transport == "wireguard" else [])
                + [{"ansible.builtin.include_role": {"name": "plan_eval_edge"}}]},
            {"hosts": "velmara_core", "gather_facts": False, "tasks":
                [applied, {"ansible.builtin.include_role": {"name": "plan_eval_core"}}]},
        ]
        (self.play_dir / "eval.yml").write_text(yaml.safe_dump(playbook, sort_keys=False))
        env = dict(os.environ, ANSIBLE_ROLES_PATH=str(self.play_dir / "roles"),
                   ANSIBLE_LOCALHOST_WARNING="false", ANSIBLE_INVENTORY_UNPARSED_WARNING="false")
        r = subprocess.run(
            ["ansible-playbook", "-i", str(self.play_dir / "inventory/peering-edges.ini"), "eval.yml",
             "-e", "ansible_connection=local", "-e", f"ansible_python_interpreter={shutil.which('python3')}",
             "-e", f"fed_work={self.work}", "-e", "fed_edge_group=velmara_edge",
             "-e", f"fed_counterparty={self.work / 'theirs.canonical.json'}", "-e", f"out={self.out}"],
            cwd=self.play_dir, env=env, capture_output=True, text=True)
        self.assertEqual(r.returncode, 0, r.stdout[-3000:] + r.stderr[-2000:])

    def test_the_deploy_input_renders_to_the_plan_hash(self):
        for variant in VARIANTS:
            with self.subTest(variant=variant):
                self.prepare(variant)
                self.build()
                for host in ("edge-a", "core-a"):
                    deploy = json.loads((self.out / f"{host}.json").read_text())
                    for k in LIVE_TOP:
                        deploy.pop(k, None)
                    for k in LIVE_HOST:
                        deploy["host"].pop(k, None)
                    planned = json.loads((self.fixture / "expected" / f"{host}.json").read_text())
                    self.assertEqual(render(deploy), render(planned),
                                     f"{variant}/{host}: the plan's ruleset hash differs from the deploy's\n"
                                     f"deploy: {json.dumps(deploy, sort_keys=True)}\n"
                                     f"plan:   {json.dumps(planned, sort_keys=True)}")


if __name__ == "__main__":
    unittest.main()
