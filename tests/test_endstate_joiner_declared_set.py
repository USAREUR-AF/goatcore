#!/usr/bin/env python3
"""A joining site's declared set is the set join-node builds (gate row I3).

`join-node` builds the data router and admit-audit at every joining site's core
node: the router in phase 13 (zenoh-deploy.yml), admit-audit with the site's
bootstrap play (site-bootstrap.yml, role admit-audit). So the catalog declares
both at every site that replicates (every_site), and a pin's `absent: true` or
the site's own flag still decides.

A site's own SPIRE server and trust domain is ADR 1164's decision, designed and
not built. The fleet pin table renders SPIRE for every member, so a joining
site is declared to run it; it does not yet, and join-node has no step for it.
Its catalog gap names that decision, not a join-node gate row: "declared, not
yet built at this site: <why> (ADR 1164)". A founder has no gap.

JOIN_NODE_BUILDS is the set `join-node` builds at a joining site's core node
(crates/goatbuilder/src/joinnode.rs, `BUILDERS`, `BOOTSTRAP_BUILDS` and
`PUBLISHERS`), with the site declaring signal federation off. The joiner is a
member of its goatnet in the pin table, as every joining site is.
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

JOIN_NODE_BUILDS = {
    "postgres-spock", "netbird-mgmt", "signal-server", "pgbackrest", "traefik", "admit-audit",
    "ory", "kratos-selfservice-ui", "zenoh-router", "mesh-coredns",
}
# Declared at every site and not built at a joining site yet, each with what closes it.
CATALOG_GAPS = {
    "netbird-dashboard": "gate row I1, I4",
    "portal": "gate row I1, I4",
    "id-admin-console": "gate row I1, I4",
    "onboard-runner": "gate row I1, I4",
    "spire": "ADR 1164",
    "spire-agent": "ADR 1164",
}


def site(tmp, joiner_vars="", members=False, deviations=None):
    """A founder `zagros` and a joiner `zagros-b`, each one host; the joiner
    optionally a member of the goatnet in the pin table."""
    root = Path(tempfile.mkdtemp(dir=tmp))
    for group in ("zagros_hosts", "zagros_b_hosts"):
        (root / "ops/ansible/group_vars" / group).mkdir(parents=True)
    (root / "ops/ansible/group_vars/zagros_hosts/vars.yml").write_text("goatnet_founder: true\n")
    (root / "ops/ansible/group_vars/zagros_b_hosts/vars.yml").write_text(
        "goatnet_founder: false\njoin_founder_site: zagros\nsignal_federation_absent_reason: a lab site\n" + joiner_vars)
    (root / "ops/ansible/inventory").mkdir(parents=True)
    (root / "ops/ansible/inventory/mgmt-hosts.ini").write_text(
        "[zagros_hosts]\nzagros-core-1 ansible_host=192.0.2.10\n[zagros_b_hosts]\nzagros-b-core-1 ansible_host=192.0.2.20\n")
    pins = ["goatnets:", "  zagros:", f"    members: [zagros_hosts{', zagros_b_hosts' if members else ''}]", "components:"]
    for c in ("postgres-spock", "zenoh-router", "spire", "spire-agent"):
        pins.append(f"  {c}:")
        dev = (deviations or {}).get(c)
        pins.append(f"    deviations: {{zagros_b_hosts: {{absent: true}}}}" if dev else "    deviations: {}")
    (root / "ops/pins").mkdir(parents=True)
    (root / "ops/pins/fleet-pins.yaml").write_text("\n".join(pins) + "\n")
    return root


def closes(gap):
    return f"gate row {gap['row']}" if gap.get("row") else gap.get("decision")


class AJoinersDeclaredSetIsWhatJoinNodeBuilds(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()

    def tearDown(self):
        self.tmp.cleanup()

    def model(self, site_name="zagros-b", **kw):
        return ENDSTATE.model(site_name, site(self.tmp.name, **kw))["services"]

    def test_the_router_and_admit_audit_are_declared_at_every_site(self):
        cat = ENDSTATE.catalog(ROOT)["services"]
        for n in ("zenoh-router", "admit-audit"):
            self.assertTrue((cat[n].get("present") or {}).get("every_site"), n)
        for members in (False, True):
            s = self.model(members=members)
            for n in ("zenoh-router", "admit-audit"):
                self.assertIs(s[n]["present"], True, (n, members))
                self.assertIsNone(s[n]["gap"], n)

    def test_a_pin_or_the_sites_flag_still_decides(self):
        s = self.model(members=True, deviations={"zenoh-router": True}, joiner_vars="admit_audit_enabled: false\n")
        self.assertIs(s["zenoh-router"]["present"], False)
        self.assertIs(s["admit-audit"]["present"], False)

    def test_spire_at_a_joiner_is_a_gap_naming_adr_1164(self):
        cat = ENDSTATE.catalog(ROOT)["services"]
        for members in (False, True):
            s = self.model(members=members)
            for n in ("spire", "spire-agent"):
                self.assertIs(s[n]["present"], True, (n, members))
                self.assertEqual(s[n]["gap"], {"row": "", "why": cat[n]["gap"]["why"], "decision": "ADR 1164"}, n)
                self.assertIn("designed, not built", s[n]["gap"]["why"], n)
        f = self.model(site_name="zagros")
        for n in ("spire", "spire-agent"):
            self.assertIs(f[n]["present"], True, n)
            self.assertIsNone(f[n]["gap"], f"{n}: a founder runs its SPIRE server")

    def test_the_grade_and_acceptance_print_what_closes_the_gap(self):
        m = ENDSTATE.model("zagros-b", site(self.tmp.name, members=True))
        g = ENDSTATE.grade(m, {"hosts": {}}, None)
        rows = {r["service"]: r["presence"] for r in g["services"]}
        path = Path(self.tmp.name) / "endstate.json"
        path.write_text(json.dumps(m))
        gaps = ACCEPTANCE.declared_gaps(str(path))
        for n, want in CATALOG_GAPS.items():
            self.assertEqual(rows[n]["status"], "GAP", n)
            self.assertTrue(rows[n]["why"].endswith(f"({want})"), rows[n]["why"])
            self.assertEqual(rows[n]["why"], ENDSTATE.gap_text(m["services"][n]["gap"]), n)
            for t in m["services"][n].get("assay") or []:
                if t in gaps:
                    self.assertTrue(gaps[t].endswith(f"({want})"), gaps[t])

    def test_a_gap_names_a_gate_row_or_a_decision(self):
        cat = ENDSTATE.catalog(ROOT)
        for n, spec in cat["services"].items():
            g = spec.get("gap")
            if g is not None:
                self.assertTrue(g.get("why") and (g.get("row") or g.get("decision")), n)

    def test_the_joiners_declared_mgmt_set_is_what_join_node_builds(self):
        s = self.model(members=True)
        declared = {n for n, v in s.items() if v["present"] is True and v["class"] == "mgmt" and not v["gap"]}
        self.assertEqual(declared, JOIN_NODE_BUILDS)
        gaps = {n: closes(v["gap"]) for n, v in s.items() if v["present"] is True and v["gap"]}
        self.assertEqual(gaps, CATALOG_GAPS)


if __name__ == "__main__":
    sys.exit(unittest.main())
