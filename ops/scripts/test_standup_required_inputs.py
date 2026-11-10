#!/usr/bin/env python3
"""Tests for standup-required-inputs.py on the incomplete fixture site and the evaluator."""
import importlib.util
import json
import subprocess
import sys
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
SCRIPT = HERE / "standup-required-inputs.py"
FIX = HERE.parent / "goatbuilder" / "fixtures" / "required-inputs"

spec = importlib.util.spec_from_file_location("sri", SCRIPT)
sri = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sri)


def scan(*extra):
    r = subprocess.run([sys.executable, str(SCRIPT), "zagros", "--ansible-dir", str(FIX / "ansible"),
                        "--plan-file", str(FIX / "plan.txt"), "--json", *extra],
                       capture_output=True, text=True, env={"PATH": "/usr/bin:/bin"})
    return r.returncode, json.loads(r.stdout)


class FixtureSite(unittest.TestCase):
    def test_every_missing_input_named_in_one_pass(self):
        rc, out = scan("--no-secrets")
        self.assertEqual(rc, 1)
        self.assertEqual(sorted(m["var"] for m in out["missing"]), ["alpha_owner", "alpha_token", "alpha_zone"])

    def test_requirements_behind_false_conditions_are_dropped(self):
        _rc, out = scan("--no-secrets")
        named = {m["var"] for m in out["missing"]} | {c["var"] for c in out["conditional"]}
        self.assertNotIn("beta_key", named)
        self.assertNotIn("alpha_optional_key", named)

    def test_non_empty_default_satisfies(self):
        _rc, out = scan("--no-secrets")
        self.assertNotIn("alpha_port", {m["var"] for m in out["missing"]})

    def test_extra_vars_from_the_plan_satisfy_mandatory(self):
        _rc, out = scan("--no-secrets")
        self.assertNotIn("target", {m["var"] for m in out["missing"]})

    def test_env_bound_secret_is_reported_when_unset(self):
        _rc, out = scan()
        self.assertIn("GALAXY_ALPHA_SECRET", {s["env"] for s in out["secrets"]})
        _rc, quiet = scan("--no-secrets")
        self.assertEqual(quiet["secrets"], [])


class PlanHeaders(unittest.TestCase):
    """Every header the driver prints is a play the scan reads; an unmatched header
    is a play whose required inputs nobody checks (the observability legs were
    dropped this way when they were renamed from 'Obs leg O1')."""

    PLAN = (
        "=== Phase 1 [GATE] config preflight ===\n    python3 preflight.py zagros_hosts\n"
        '=== Observability leg "host" [A \u2225] observability host: baseline ===\n'
        "    ansible-playbook obs-host-baseline.yml -e target=zagros_obs_hosts\n"
        "=== Obs leg O3 [A \u2225] the old header ===\n    ansible-playbook obs.yml\n"
    )

    def test_observability_legs_are_read(self):
        import tempfile
        with tempfile.NamedTemporaryFile("w", suffix=".txt", delete=False) as f:
            f.write(self.PLAN)
        phases = sri.read_plan("zagros", None, f.name)
        self.assertEqual([p["phase"] for p in phases],
                         ["1", "observability leg host", "observability leg O3"])
        self.assertTrue(all(p["cmd"] for p in phases))


class Evaluator(unittest.TestCase):
    def ev(self, cond, vals):
        return sri.evaluate(cond, vals)

    def test_bool_and_default(self):
        self.assertFalse(self.ev("beta_enabled | default(false) | bool", {"beta_enabled": False}))
        self.assertTrue(self.ev("x | default(true) | bool", {}))

    def test_equality_and_not(self):
        self.assertTrue(self.ev("mgmt_stack_profile == 'full'", {"mgmt_stack_profile": "full"}))
        self.assertTrue(self.ev("not (skip | bool)", {"skip": False}))

    def test_length(self):
        self.assertFalse(self.ev("(svc | default([]) | length) > 0", {"svc": []}))

    def test_unknown_is_not_guessed(self):
        with self.assertRaises(sri.Unknown):
            self.ev("result.rc == 0", {})
        with self.assertRaises(sri.Unknown):
            self.ev("undefined_thing | bool", {})


class Classify(unittest.TestCase):
    def test_shapes(self):
        self.assertEqual(sri.classify("x | length > 0"), [("x", "nonempty")])
        self.assertEqual(sri.classify("x is defined and (x | length) >= 16"), [("x", "defined"), ("x", "nonempty")])
        self.assertEqual(sri.classify("x | default('', true) | length > 0"), [("x", "nonempty")])
        self.assertEqual(sri.classify("x != ''"), [("x", "nonempty")])
        self.assertEqual(sri.classify("x.status == 200"), [])

    def test_conditional_mandatory_is_not_unconditional(self):
        self.assertEqual(sri.mandatory_vars("- not (a | default(false) and (z | mandatory | length) == 0)"), [])
        self.assertEqual(sri.mandatory_vars('hosts: "{{ t | mandatory }}"'), ["t"])

    def test_an_env_lookup_that_tests_the_run_is_not_a_need(self):
        # group_vars/all.yml's aws_cmd asks whether the run is inside aws-vault; an
        # internet site's relay plays reach it, and AWS_VAULT is not a secret to supply.
        aws_cmd = ("{% if lookup('env', 'AWS_VAULT') %}{% else %}aws-vault exec "
                   "{{ aws_profile | mandatory }} --no-session -- {% endif %}")
        self.assertFalse(sri.env_lookup_is_a_need(aws_cmd))
        self.assertFalse(sri.env_lookup_is_a_need("{{ lookup('env', 'GHCR_ORG') | default('x') }}"))
        self.assertFalse(sri.env_lookup_is_a_need("{{ lookup('env', 'SRC') or 'goat' }}"))
        self.assertTrue(sri.env_lookup_is_a_need("{{ lookup('env', 'CLOUDFLARE_API_TOKEN_X') }}"))
        self.assertTrue(sri.env_lookup_is_a_need("{{ lookup('env', 'X_PASS') | default('', true) }}"))


class Leaves(unittest.TestCase):
    """#2165: a reached play's controller-held leaf is checked where goat_leaf looks."""

    class G:
        def __init__(self, roles=(), plays=()):
            self.roles = {r: [[]] for r in roles}
            self.plays = [(p, {}, []) for p in plays]

    def test_router_lan_and_collector_leaves_are_named(self):
        g = self.G(roles=["goat-zenoh", "names-collector"], plays=["lan-traefik-routes.yml"])
        need = sri.leaves_needed(g, {"names_collector_cert_src": "zagros/names-collector-cert.pem",
                                     "names_collector_key_src": "zagros/names-collector-key.pem"}, "zagros")
        self.assertEqual([n for n, _ in need], [
            "zagros/router-cert.pem", "zagros/router-key.pem",
            "zagros/mgmt-identity-cert.pem", "zagros/mgmt-identity-key.pem",
            "zagros/names-collector-cert.pem", "zagros/names-collector-key.pem"])

    def test_a_site_serving_another_sites_router_leaf(self):
        need = sri.leaves_needed(self.G(roles=["goat-zenoh"]), {"goat_router_leaf_site": "zagros"}, "zagros-cloud")
        self.assertEqual(need[0][0], "zagros/router-cert.pem")

    def test_a_templated_site_falls_back_to_site_id(self):
        need = sri.leaves_needed(self.G(roles=["goat-zenoh"]), {"goat_router_leaf_site": "{{ site_id }}"}, "zagros")
        self.assertEqual(need[0][0], "zagros/router-cert.pem")

    def test_nothing_reached_needs_nothing(self):
        self.assertEqual(sri.leaves_needed(self.G(roles=["isitup"]), {}, "zagros"), [])


class ProducedByThisRun(unittest.TestCase):
    """What a later step of the same run produces is reported, not refused; what nothing
    produces, or what an earlier step reads first, is still a refusal."""

    STACK = "=== Phase 5 [A] fixture stack ===\n    cd ops/ansible && ../scripts/ansible-preflight.sh -i inventory/hosts.ini site.yml -e target=zagros_hosts\n"
    GATE = "=== Phase 1 [GATE] config preflight (fail-closed) ===\n    python3 preflight.py zagros_hosts\n"

    def manual(self, n, name):
        return f"=== Phase {n} [MANUAL] the browser step ===\n    create its token, and store it as {name} under the declared secrets posture\n"

    def run_plan(self, plan):
        import tempfile
        with tempfile.NamedTemporaryFile("w", suffix=".txt", delete=False) as f:
            f.write(plan)
        r = subprocess.run([sys.executable, str(SCRIPT), "zagros", "--ansible-dir", str(FIX / "ansible"),
                            "--plan-file", f.name, "--json"], capture_output=True, text=True, env={"PATH": "/usr/bin:/bin"})
        return json.loads(r.stdout)

    def test_a_secret_a_step_before_its_reader_stores_is_produced(self):
        out = self.run_plan(self.GATE + self.manual(3, "GALAXY_ALPHA_SECRET") + self.STACK)
        self.assertNotIn("GALAXY_ALPHA_SECRET", {s["env"] for s in out["secrets"]})
        self.assertEqual([(p["name"], p["phase"], p["kind"]) for p in out["produced"]], [("GALAXY_ALPHA_SECRET", "3", "secret")])

    def test_a_secret_produced_after_its_reader_is_still_a_refusal(self):
        out = self.run_plan(self.GATE + self.STACK + self.manual(11, "GALAXY_ALPHA_SECRET"))
        row = next(s for s in out["secrets"] if s["env"] == "GALAXY_ALPHA_SECRET")
        self.assertIn("read before phase 11, which produces it", row["required_by"])
        self.assertEqual(out["produced"], [])

    def run_range(self, plan, *rng):
        import tempfile
        with tempfile.NamedTemporaryFile("w", suffix=".txt", delete=False) as f:
            f.write(plan)
        r = subprocess.run([sys.executable, str(SCRIPT), "zagros", "--ansible-dir", str(FIX / "ansible"),
                            "--plan-file", f.name, "--json", *rng], capture_output=True, text=True, env={"PATH": "/usr/bin:/bin"})
        return r.returncode, json.loads(r.stdout)

    def test_the_phase_1_gate_up_to_11_passes_what_phase_3_produces(self):
        plan = self.GATE + self.manual(3, "GALAXY_ALPHA_SECRET") + self.STACK + self.manual(11, "ZAGROS_NB_API_TOKEN")
        _rc, out = self.run_range(plan, "--to", "11", "--no-secrets")
        self.assertEqual(out["missing"] and [m["var"] for m in out["missing"]], ["alpha_owner", "alpha_token", "alpha_zone"],
                         "the fixture's real gaps still refuse inside the range")
        _rc, out = self.run_range(plan, "--to", "11")
        self.assertNotIn("GALAXY_ALPHA_SECRET", {s["env"] for s in out["secrets"]})
        self.assertIn(("GALAXY_ALPHA_SECRET", "3"), {(p["name"], p["phase"]) for p in out["produced"]})

    def test_a_producer_outside_the_scanned_range_counts_for_nothing(self):
        plan = self.GATE + self.manual(3, "GALAXY_ALPHA_SECRET") + self.STACK
        _rc, out = self.run_range(plan, "--from", "5")
        self.assertIn("GALAXY_ALPHA_SECRET", {s["env"] for s in out["secrets"]})
        self.assertEqual(out["produced"], [])

    def test_a_secret_nothing_produces_is_still_a_refusal(self):
        out = self.run_plan(self.GATE + self.manual(3, "ZAGROS_SOMETHING_ELSE") + self.STACK)
        self.assertIn("GALAXY_ALPHA_SECRET", {s["env"] for s in out["secrets"]})
        self.assertEqual(out["produced"], [])

    def test_only_a_step_that_stores_the_name_counts_as_producing_it(self):
        self.assertEqual(sri.MANUAL_STORES.findall("read $OWNER_EMAIL, then ZAGROS_NB_API_TOKEN is checked"), [])
        self.assertEqual(sri.MANUAL_STORES.findall("add 'export ZAGROS_NB_API_TOKEN=<token>' to a file"), ["ZAGROS_NB_API_TOKEN"])

    def test_phase_3_is_promised_only_where_it_can_write(self):
        self.assertTrue(sri.phase3_can_write("env-file", {}))
        self.assertFalse(sri.phase3_can_write("env", {}), "the env store persists nothing")
        self.assertFalse(sri.phase3_can_write("1password", {}), "no vault named")
        self.assertTrue(sri.phase3_can_write("1password", {"GOAT_SECRET_VAULT": "v"}))
        self.assertFalse(sri.phase3_can_write("custom", {}), "no adapter")
        self.assertFalse(sri.phase3_can_write("vaultwarden", {}), "the generator does not write there")

    def test_a_joiner_counts_no_phase_3_secret(self):
        plan = [{"phase": "3", "kind": "T", "label": "generate per-instance secrets", "cmd": "'/r/ops/scripts/newsite-secrets.sh' uk"},
                {"phase": "6", "kind": "A", "label": "[joiner] seed-from-peer", "cmd": "true"}]
        self.assertEqual(sri.LaterSteps(plan, "uk", "env-file").secrets, {})
        founder = [p for p in plan if "joiner" not in p["label"]]
        self.assertIn("UK_PG_PASS", sri.LaterSteps(founder, "uk", "env-file").secrets)

    def test_the_bootstrap_pair_is_not_promised(self):
        names = sri.phase3_names("zagros")
        self.assertIn("ZAGROS_PG_PASS", names)
        self.assertNotIn("ZAGROS_PORTAL_OPERATOR_API_KEYS", names)

    def manifest(self, san="192.0.2.50", ca=True):
        import tempfile
        root = Path(tempfile.mkdtemp())
        (root / "ops/enrollment/manifests").mkdir(parents=True)
        key = root / "ca.key.age"
        cert = root / "root-cert.pem"
        if ca:
            key.write_text("x")
            cert.write_text("x")
        (root / "ops/enrollment/manifests/galaxy-certs.yaml").write_text(yaml_doc(cert, key, san))
        return root

    def test_a_leaf_in_the_manifest_is_minted(self):
        got = sri.minted_leaves("galaxy", "mint-site-certs.sh galaxy", {}, self.manifest())
        self.assertEqual(got, {"galaxy/router-cert.pem", "galaxy/router-key.pem"})

    def test_a_missing_ca_mints_nothing_unless_the_plan_mints_one(self):
        root = self.manifest(ca=False)
        self.assertEqual(sri.minted_leaves("galaxy", "mint-site-certs.sh galaxy", {}, root), set())
        self.assertIn("galaxy/router-key.pem", sri.minted_leaves("galaxy", "mint-site-certs.sh galaxy --mint-ca-if-missing", {}, root))

    def test_half_a_ca_mints_nothing(self):
        root = self.manifest(ca=False)
        (root / "ca.key.age").write_text("x")
        self.assertEqual(sri.minted_leaves("galaxy", "mint-site-certs.sh galaxy --mint-ca-if-missing", {}, root), set())

    def test_an_unresolved_san_mints_nothing(self):
        root = self.manifest(san="TBD_ZAGROS_MESH_IP_A")
        self.assertEqual(sri.minted_leaves("galaxy", "mint-site-certs.sh galaxy", {}, root), set())
        self.assertIn("galaxy/router-cert.pem", sri.minted_leaves("galaxy", "mint-site-certs.sh galaxy", {"mesh_ip_a": "192.0.2.50"}, root))

    def test_no_manifest_mints_nothing(self):
        import tempfile
        self.assertEqual(sri.minted_leaves("galaxy", "mint-site-certs.sh galaxy", {}, Path(tempfile.mkdtemp())), set())


def yaml_doc(cert, key, san):
    return f"site_id: galaxy\nca:\n  root_cert: {cert}\n  root_key: {key}\nleaves:\n  - name: router\n    sans:\n      - {san}\n"


class LsGuard(unittest.TestCase):
    def test_a_play_behind_an_absent_file_is_not_reached(self):
        cmd = "cd ops/ansible && ( ls group_vars/nowhere/pubsub-state.yml >/dev/null 2>&1 && ../scripts/ansible-preflight.sh site.yml -e target=zagros_hosts || echo no )"
        self.assertEqual(sri.plays_in(cmd, "zagros_hosts"), [])


if __name__ == "__main__":
    unittest.main()
