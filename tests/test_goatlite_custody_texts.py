#!/usr/bin/env python3
"""What a goatlite's standup tells the learner about its secrets is true for a goatlite.

A goatlite has only goatlite.yaml and no outside accounts. Phase 1's
`posture.py init --if-env-file` runs only when env-file is already the store in
force, so it gives no advice to declare one in goatbuilder.yaml. Phase 6's
bootstrap setup-key message names where a goatlite keeps the key, not a
password manager it does not have. The full preset's text is unchanged in both.
"""
import contextlib
import importlib.util
import io
from pathlib import Path
import sys
import tempfile
import unittest

import jinja2
import yaml

ROOT = Path(__file__).resolve().parents[1]


def load(name, rel):
    spec = importlib.util.spec_from_file_location(name, ROOT / rel)
    mod = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = mod
    spec.loader.exec_module(mod)
    return mod


posture = load("goat_posture_custody", "ops/secrets/posture.py")
render = load("goatlite_render_custody", "ops/scripts/goatlite-render.py")

DECLARE = "declare the posture: secrets: {store: env-file} in ops/goatbuilder/goatbuilder.yaml"


class Phase1Advice(unittest.TestCase):
    def init_output(self, if_env_file):
        saved = posture.store, posture.bindings_for
        posture.store = lambda repo=None: "env-file"
        posture.bindings_for = lambda goatnet, repo=None: ["FIXTURE_DB_PASS"]
        try:
            with tempfile.TemporaryDirectory() as d:
                out = io.StringIO()
                with contextlib.redirect_stdout(out):
                    rc = posture.init("fixture", str(Path(d) / "secrets.env"), force=True, if_env_file=if_env_file)
                self.assertEqual(rc, 0)
                return out.getvalue()
        finally:
            posture.store, posture.bindings_for = saved

    def test_a_goatlite_phase_1_gives_no_goatbuilder_yaml_advice(self):
        out = self.init_output(if_env_file=True)
        self.assertIn("secrets init: ", out)
        self.assertNotIn(DECLARE, out)
        self.assertNotIn("goatbuilder.yaml", out)

    def test_the_full_presets_init_still_says_how_to_declare_the_store(self):
        self.assertIn(DECLARE, self.init_output(if_env_file=False))


def setup_key_message(extra_vars):
    """The phase-6 'show bootstrap setup key' message, templated as ansible
    does (trim_blocks), with the vars a run would carry."""
    plays = yaml.safe_load((ROOT / "ops/ansible/site-bootstrap.yml").read_text())

    def find(node):
        if isinstance(node, dict):
            if node.get("name") == "show bootstrap setup key":
                return node["ansible.builtin.debug"]["msg"]
            node = list(node.values())
        if isinstance(node, list):
            for n in node:
                m = find(n)
                if m is not None:
                    return m
        return None

    msg = find(plays)
    assert msg is not None, "site-bootstrap.yml has no 'show bootstrap setup key' task"
    env = jinja2.Environment(trim_blocks=True, undefined=jinja2.ChainableUndefined,
                             autoescape=jinja2.select_autoescape(default_for_string=False))
    base = {"site_id": "fixture", "site_account_id": "acc_fixture", "create_key": {"stdout": "setup_key_id: sk_x"}}
    return env.from_string(msg).render(**base, **extra_vars)


class Phase6Custody(unittest.TestCase):
    def lite_vars(self):
        doc = yaml.safe_load(render.vars_yml(render.facts(render.load_config(self.config(), "fixture"))))
        return doc

    def config(self):
        d = tempfile.TemporaryDirectory()
        self.addCleanup(d.cleanup)
        pub = Path(d.name) / "id.pub"
        pub.write_text("placeholder\n")
        cfg = Path(d.name) / "goatlite.yaml"
        cfg.write_text(yaml.safe_dump({"schema": "goatbuilder/v1", "preset": "lite",
                                       "lite": {"name": "fixture", "host": "192.0.2.50", "ssh_user": "ubuntu",
                                                "admin": "admin@example.org", "ssh_pubkey": str(pub)}}))
        return cfg

    def test_a_goatlite_is_told_to_keep_the_key_in_its_local_store(self):
        v = self.lite_vars()
        self.assertIn("setup_key_custody", v)
        msg = setup_key_message({"setup_key_custody": v["setup_key_custody"]})
        self.assertNotIn("1Password", msg)
        self.assertIn("local secrets store (goatbuilder secrets store names it)", msg)
        self.assertIn("Revoke within 24 h via:", msg)

    def test_the_full_preset_keeps_its_password_manager_line(self):
        msg = setup_key_message({})
        self.assertIn('Copy to 1Password as\n  "goat / fixture / bootstrap key / <today>"\nRevoke within 24 h via:', msg)
        self.assertNotIn("{%", msg)


if __name__ == "__main__":
    unittest.main()
