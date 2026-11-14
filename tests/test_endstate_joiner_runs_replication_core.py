#!/usr/bin/env python3
"""A joiner's end state marks present every service a live joiner runs.

The gate of ADR 1233 (docs/design/goatbuilder.md, `join-node` part (e)): "the
model of a joiner fixture marks present every service a live joiner runs, and
acceptance with --endstate reads no row N/A for one of them".

What a live joiner runs was measured on 2026-11-13 by reading the running
containers on the management hosts of two joined sites of a standing goatnet.
Both run the services in LIVE_JOINER_RUNS. Neither runs the NetBird dashboard,
the portal, the id console, the onboarding engine or mesh DNS yet.

Every site is declared fully island-mode capable (ADR 1165, gate group I), so
the end state declares every one of those present at every site that
replicates. What a joining site does not run yet is a named gap, "declared, not
yet built at this site", with the gate row that closes it (GAPS_AT_A_JOINER),
never an absence by design and never N/A.

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
# Run by neither joiner yet: each a gap, with the gate row that closes it. The
# admin surfaces wait for the multi-writer schema (I1) and their write class's
# switch (I4); mesh DNS waits for join-node to deploy it (I3).
GAPS_AT_A_JOINER = {
    "netbird-dashboard": "I1, I4",
    "portal": "I1, I4",
    "id-admin-console": "I1, I4",
    "onboard-runner": "I1, I4",
    "mesh-coredns": "I3",
}
# The services a site cut off from the others needs, declared at every site.
EVERY_SITE = ("traefik", "netbird-mgmt", "signal-server", "ory", "kratos-selfservice-ui")

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

    def test_what_a_joiner_does_not_run_yet_is_a_named_gap(self):
        s = self.model["services"]
        g = ENDSTATE.grade(self.model, {"hosts": {}}, None)
        rows = {r["service"]: r for r in g["services"]}
        for n, row in GAPS_AT_A_JOINER.items():
            self.assertIs(s[n]["present"], True, f"{n} is declared at every site")
            self.assertEqual((s[n]["gap"] or {}).get("row"), row, n)
            self.assertIsNone(s[n]["reason"], n)
            p = rows[n]["presence"]
            self.assertEqual(p["status"], "GAP", n)
            self.assertTrue(p["why"].startswith("declared, not yet built at this site: "), p["why"])
            self.assertTrue(p["why"].endswith(f"(gate row {row})"), p["why"])
        for r in g["services"]:
            for wrong in ("founder-only", "single-homed", "by design"):
                self.assertNotIn(wrong, r["presence"]["why"], r["service"])

    def test_acceptance_reads_a_gap_as_a_gap_never_na(self):
        path = Path(self.tmp.name) / "endstate.json"
        path.write_text(json.dumps(self.model))
        absent = ACCEPTANCE.declared_absent(str(path))
        gaps = ACCEPTANCE.declared_gaps(str(path))
        for n, row in GAPS_AT_A_JOINER.items():
            for t in self.model["services"][n].get("assay") or []:
                self.assertNotIn(t, absent, f"{n}'s row {t} would read N/A")
                self.assertIn(t, gaps, n)
                self.assertTrue(gaps[t].startswith("declared, not yet built at this site: "), gaps[t])
                self.assertIn(f"gate row {row}", gaps[t])

    def test_no_catalog_row_is_founder_only(self):
        cat = ENDSTATE.catalog(ROOT)["services"]
        for n, spec in cat.items():
            self.assertNotIn("founder", spec.get("present") or {}, f"{n}: founder-only is not a presence rule")
            self.assertNotIn("admin_surface", spec, n)
        for n in list(EVERY_SITE) + list(GAPS_AT_A_JOINER):
            self.assertTrue(
                (cat[n].get("present") or {}).get("every_site"),
                f"{n} is declared at every site that replicates",
            )

    def test_the_gap_wording_comes_from_the_catalog(self):
        cat = ENDSTATE.catalog(ROOT)["services"]
        for n in GAPS_AT_A_JOINER:
            self.assertEqual(self.model["services"][n]["gap"], {"row": cat[n]["gap"]["row"], "why": cat[n]["gap"]["why"]})
        for name in ("D3_ADMIN_SURFACES", "D3_REASON"):
            self.assertFalse(hasattr(ENDSTATE, name), f"{name}: the wording is the catalog's, not code's")

    def test_a_founder_is_graded_the_same_way(self):
        m = ENDSTATE.model("zagros", self.root)["services"]
        for n in list(EVERY_SITE) + list(GAPS_AT_A_JOINER):
            self.assertIs(m[n]["present"], True, n)
            self.assertIsNone(m[n]["gap"], f"{n}: a founder that runs it has no gap")


if __name__ == "__main__":
    sys.exit(unittest.main())
