#!/usr/bin/env python3
"""A joiner's end state marks present every service a live joiner runs.

The gate of ADR 1233 (docs/design/goatbuilder.md, `join-node` part (e)): "the
model of a joiner fixture marks present every service a live joiner runs, and
acceptance with --endstate reads no row N/A for one of them".

What a live joiner runs was measured on 2026-11-13 by reading the running
containers on the management hosts of two joined sites of a standing goatnet.
Both run the services in LIVE_JOINER_RUNS. Neither runs the NetBird dashboard,
the portal, the id console, the onboarding engine or mesh DNS, so the fixture's
declarations leave those off, and the model must give each its reason.

federation-proxy also runs on both joiners; the catalog has no row for it, so
the model cannot mark it either way.
"""
import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def _load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


ENDSTATE = _load("endstate", ROOT / "ops/goatbuilder/endstate/endstate.py")
ACCEPTANCE = _load("run_acceptance", ROOT / "ops/validation/run-acceptance.py")

# The catalog services behind the containers both live joiners run.
LIVE_JOINER_RUNS = (
    "admit-audit",
    "consistency-health",
    "zenoh-router",
    "fabric-crawler",
    "fabric-health",
    "netbird-mgmt",
    "mesh-health",
    "ory",
    "postgres-spock",
    "presence-health",
    "kratos-selfservice-ui",
    "signal-server",
    "time-health",
    "traefik",
)
# Run by neither joiner.
LIVE_JOINER_DOES_NOT_RUN = ("netbird-dashboard", "portal", "id-admin-console", "onboard-runner", "mesh-coredns")

ADMIN_SURFACES = ("netbird-dashboard", "portal", "id-admin-console", "onboard-runner")
D3 = "single-homed at the founder (ADR 1084 §D3); every site once ADR 1165 lifts it for this class"

JOINER_VARS = """\
goatnet_founder: false
join_founder_site: zagros
admit_audit_enabled: true
consistency_health_enabled: true
id_admin_console_enabled: false
onboard_runner_enabled: false
"""

# The pinned services a joiner runs are rendered for it; the portal and the
# onboarding engine are not.
PINNED = sorted(
    {"netbird-mgmt", "signal-server", "kratos-selfservice-ui", "postgres-spock", "zenoh-router", "fabric-health",
     "fabric-crawler", "mesh-health", "presence-health", "time-health", "consistency-health", "portal",
     "onboard-runner"}
)
NOT_RENDERED_FOR_THE_JOINER = ("portal", "onboard-runner")


def joiner_fixture(tmp):
    root = Path(tmp)
    for group in ("zagros_hosts", "zagros_b_hosts"):
        (root / "ops/ansible/group_vars" / group).mkdir(parents=True)
    (root / "ops/ansible/group_vars/zagros_hosts/vars.yml").write_text("goatnet_founder: true\n")
    (root / "ops/ansible/group_vars/zagros_b_hosts/vars.yml").write_text(JOINER_VARS)
    lines = ["goatnets:", "  zagros:", "    members: [zagros_hosts, zagros_b_hosts]", "components:"]
    for c in PINNED:
        lines.append(f"  {c}:")
        if c in NOT_RENDERED_FOR_THE_JOINER:
            lines.append("    deviations:")
            lines.append("      zagros_b_hosts: {absent: true}")
        else:
            lines.append("    deviations: {}")
    (root / "ops/pins").mkdir(parents=True)
    (root / "ops/pins/fleet-pins.yaml").write_text("\n".join(lines) + "\n")
    return root


class AJoinerRunsTheReplicationCore(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = joiner_fixture(self.tmp.name)
        self.model = ENDSTATE.model("zagros-b", self.root)

    def tearDown(self):
        self.tmp.cleanup()

    def test_the_model_marks_present_every_service_a_live_joiner_runs(self):
        s = self.model["services"]
        wrong = {n: (s[n]["present"], s[n]["reason"]) for n in LIVE_JOINER_RUNS if s[n]["present"] is not True}
        self.assertEqual(wrong, {}, "a live joiner runs these, so the model must mark them present")

    def test_acceptance_with_endstate_reads_no_row_na_for_one_of_them(self):
        path = Path(self.tmp.name) / "endstate.json"
        path.write_text(json.dumps(self.model))
        absent_tokens = ACCEPTANCE.declared_absent(str(path))
        run_tokens = {t for n in LIVE_JOINER_RUNS for t in (self.model["services"][n].get("assay") or [])}
        self.assertTrue(run_tokens, "the live joiner's services own acceptance rows")
        self.assertEqual(absent_tokens & run_tokens, set(), "rows a live joiner's services own would read N/A")

    def test_what_neither_joiner_runs_is_absent_with_its_reason(self):
        s = self.model["services"]
        for n in LIVE_JOINER_DOES_NOT_RUN:
            self.assertIs(s[n]["present"], False, n)
            self.assertTrue(s[n]["reason"], n)
        for n in ADMIN_SURFACES:
            self.assertTrue(s[n]["reason"].startswith(D3), f"{n}: {s[n]['reason']}")
        self.assertFalse(s["mesh-coredns"]["reason"].startswith(D3), "mesh DNS is not an admin surface")

    def test_the_admin_surface_wording_comes_from_the_catalog_mark(self):
        cat = ENDSTATE.catalog(ROOT)["services"]
        marked = sorted(n for n, spec in cat.items() if spec.get("admin_surface"))
        self.assertEqual(marked, sorted(ADMIN_SURFACES))
        self.assertFalse(hasattr(ENDSTATE, "D3_ADMIN_SURFACES"), "the admin surfaces are named in the catalog, not in code")

    def test_a_founder_is_graded_the_same_way_for_the_replication_core(self):
        m = ENDSTATE.model("zagros", self.root)["services"]
        for n in ("traefik", "netbird-mgmt", "signal-server", "ory", "kratos-selfservice-ui"):
            self.assertIs(m[n]["present"], True, n)
        self.assertIs(m["netbird-dashboard"]["present"], True)
        self.assertIs(m["mesh-coredns"]["present"], True)


if __name__ == "__main__":
    sys.exit(unittest.main())
