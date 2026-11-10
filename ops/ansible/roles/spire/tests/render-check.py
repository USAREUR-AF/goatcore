#!/usr/bin/env python3
"""Render spire-trust-sync.sh.j2 and assert per-partner trust isolation.

WHAT THIS PROVES THAT THE CI GUARD CANNOT. The guard reads the template source
and the group_vars declarations. This renders the script the way Ansible renders
it, with TWO declared partners, and asserts the SHELL that comes out fetches one
bundle per trust domain — the property the defect violated. A template can hold
`bundle list -id` in a branch that never renders; only a render shows what the
host actually runs.

The federation blocks are Jinja-gated on spire_federation_enabled, so this also
covers the OFF case: with federation off, no federated content is emitted at all.

    python3 ops/ansible/roles/spire/tests/render-check.py

Exit 0 = pass, 1 = fail, 2 = jinja2 unavailable (CI installs it).
"""
from __future__ import annotations

import re
import sys
from pathlib import Path

try:
    from jinja2 import Environment, FileSystemLoader
except ImportError:
    print("ERROR: jinja2 required (pip install jinja2)", file=sys.stderr)
    sys.exit(2)

TPL_DIR = Path(__file__).resolve().parent.parent / "templates"
TPL = "spire-trust-sync.sh.j2"

CORE_TRUST = "/etc/zagros-zenoh/tls/ca-root-cert.pem"
EDGE_A = "/etc/goat-zenoh-edge-aaa/tls/partner-ca.pem"
EDGE_B = "/etc/goat-zenoh-edge-bbb/tls/partner-ca.pem"
DOMAIN_A = "aaa.partner.example"
DOMAIN_B = "spiffe://bbb.partner.example"   # the other accepted spelling
EDGE_R = "/etc/goat-zenoh-edge-rrr/tls/partner-ca.pem"
DOMAIN_R = "rrr.partner.example"


def render(**overrides) -> str:
    # trim_blocks matches Ansible's template module, so this render matches what
    # the role writes to /usr/local/sbin/spire-trust-sync.sh.
    env = Environment(loader=FileSystemLoader(str(TPL_DIR)), trim_blocks=True)
    base = dict(
        ansible_managed="ANSIBLE MANAGED",
        spire_data_dir="/opt/spire/data",
        spire_compose_dir="/opt/mgmt-host",
        spire_bin_dir="/opt/spire/bin",
        spire_trust_domain="zagros.goat.example",
        spire_agent_node_path="agent/thor",
        spire_agent_rescue_enabled=True,
        spire_trust_sync_interval="15min",
        spire_trust_sync_router_trust_file=CORE_TRUST,
        spire_trust_sync_restart_containers=["zagros-zenoh-router"],
        spire_trust_sync_federated_edges=[],
        spire_trust_sync_federated_edges_retired=[],
        spire_trust_sync_prune_enabled=True,
        spire_federation_enabled=False,
    )
    base.update(overrides)
    return env.get_template(TPL).render(**base)


fails = 0


def check(cond, msg):
    global fails
    print(("  PASS: " if cond else "  FAIL: ") + msg)
    if not cond:
        fails += 1


def strip_comments(s: str) -> str:
    return "\n".join(ln for ln in s.splitlines() if not ln.lstrip().startswith("#"))


print("federation OFF (every goatnet today):")
off = strip_comments(render())
check("bundle list" not in off, "no federated bundle fetch is emitted at all")
check("federated_edges" not in off, "no peering table is emitted")
check(CORE_TRUST in off, "the core router trust file is still written")
check("retired_edges" not in off, "no retired-peering table is emitted")

print("federation ON, two declared partners:")
on_raw = render(
    spire_federation_enabled=True,
    spire_trust_sync_federated_edges=[
        {"trust_file": EDGE_A, "trust_domain": DOMAIN_A},
        {"trust_file": EDGE_B, "trust_domain": DOMAIN_B},
    ],
)
on = strip_comments(on_raw)

# THE PROPERTY. The fetch is scoped to one trust domain, and the all-domains
# form (`bundle list` with no -id) appears nowhere in the executable script.
check('bundle list -id "$_td_id"' in on,
      "edge bundles are fetched per trust domain (bundle list -id)")
check(re.search(r"bundle list(?!\s+-id)", on) is None,
      "no unqualified `bundle list` — an edge never receives all partners")

# The peering table must carry BOTH partners, each on its own line pairing the
# domain with its own file. A line pairing A's domain with B's file would be the
# defect surviving the refactor.
table = re.search(r"cat <<'EDGES'\n(.*?)\nEDGES", on, re.S)
check(table is not None, "the peering table renders as a quoted heredoc")
if table:
    lines = [ln for ln in table.group(1).splitlines() if ln.strip()]
    check(lines == [f"{DOMAIN_A} {EDGE_A}", f"{DOMAIN_B} {EDGE_B}"],
          f"one line per peering, domain bound to its OWN file: {lines}")

# The core/edge separation the F-424 mitigation already had. $tmp is the own
# bundle; it must be what reaches the core, and the edge write must not be it.
check('append_certs "$tmp" "$ROUTER_TRUST_FILE"' in on,
      "the CORE router still receives the OWN bundle only ($tmp)")
check('append_certs "$edge_tmp" "$_ftf"' in on,
      "each edge receives its own per-peering bundle ($edge_tmp)")
check(CORE_TRUST not in table.group(1) if table else False,
      "the core trust file is not in the peering table")

# A declared partner with no bundle yet is normal before federation completes,
# and must be logged rather than fatal.
check("no federated bundle for" in on_raw,
      "a partner with no bundle yet is logged, not fatal")
check("exit 1" not in on.split("while read -r _td _ftf")[-1].split("done <")[0],
      "the edge loop has no fatal exit on a missing partner bundle")

# --- retirement + reconciliation --------------------------------------------
# Retiring a peering is a DECLARED change: the entry moves from the active list
# to the retired one, and the retired pass converges that trust file on our own
# bundle alone. The behavioural proof that a certificate actually disappears is
# trust-sync-reconcile-test.sh; these assert the shape the host runs.
print("federation ON, one peering retired:")
ret = strip_comments(render(
    spire_federation_enabled=True,
    spire_trust_sync_federated_edges=[
        {"trust_file": EDGE_A, "trust_domain": DOMAIN_A},
    ],
    spire_trust_sync_federated_edges_retired=[
        {"trust_file": EDGE_R, "trust_domain": DOMAIN_R},
    ],
))

rtable = re.search(r"cat <<'RETIRED'\n(.*?)\nRETIRED", ret, re.S)
check(rtable is not None, "the retired-peering table renders as a quoted heredoc")
if rtable:
    rlines = [ln for ln in rtable.group(1).splitlines() if ln.strip()]
    check(rlines == [f"{DOMAIN_R} {EDGE_R}"],
          f"the retired peering is named with its own file: {rlines}")
    check(EDGE_A not in rtable.group(1),
          "an ACTIVE edge is not also in the retired table")
    check(CORE_TRUST not in rtable.group(1),
          "the core trust file is not in the retired table")

# A retired edge is reconciled against $tmp — our OWN bundle alone, which is the
# only declared set that is guaranteed present, so retirement converges on every
# tick rather than only when a fetch happens to succeed.
check('reconcile_certs "$tmp" "$_ftf"' in ret,
      "a retired edge is reconciled against our own bundle alone")

# THE PROPERTY THAT KEEPS REMOVAL SAFE. Reconciliation of an ACTIVE edge runs
# only when that partner's bundle was actually fetched this tick; otherwise the
# script falls back to appending, so a transient failure cannot reduce trust.
check('[ "$_partner_ok" = "1" ]' in ret,
      "active-edge reconciliation is gated on the partner bundle being fetched")
check('append_certs "$edge_tmp" "$_ftf"' in ret,
      "the fetch-failure fallback is the append path, which never removes")

# The CORE is append-only in every render. It legitimately carries CAs this role
# does not declare; rewriting it from the declared set would remove them.
check(re.search(r'reconcile_certs "[^"]*" "\$ROUTER_TRUST_FILE"', ret) is None,
      "the CORE router trust file is never reconciled")
check('append_certs "$tmp" "$ROUTER_TRUST_FILE"' in ret,
      "the CORE router trust file is still appended to, own bundle only")

# Removing a certificate authority from a live edge must be inspectable first.
check("DRY_RUN=1" in ret and "--dry-run" in ret,
      "the script accepts --dry-run")

print("FAIL" if fails else "PASS")
sys.exit(1 if fails else 0)
