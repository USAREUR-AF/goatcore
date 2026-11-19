#!/usr/bin/env python3
"""GHSA-rr3p: a slot's earlier leaves, and an unassigned slot's leaves, hold
nothing on the router.

Renders config.json5.j2 with the slot fragment the portal would write at each
step of the advisory's sequence and decides each peer's access with
files/acl-evaluate.py (the router's own decision rules, no live zenohd):

  1. holder A's leaf (generation 1) writes its namespace and reads the fabric;
  2. A is offboarded and the slot is reassigned to B (generation 3): A's leaf is
     refused for write and read, B's works;
  3. B rotates (generation 4): B's previous leaf is refused, the new one works;
  4. a leaf for an index nobody holds is refused;
  5. a leaf minted before generations existed (CN = bare anchor) is refused once
     its slot has moved on.

Run for the default read (partition off) and with the partner partition and
the scoped read on. The same sequence against a real zenohd 1.9.0, including
the host reload watcher, is tests/rr3p-live-proof.sh (docker).

    python3 ops/ansible/roles/goat-zenoh/tests/rr3p-check.py
"""
import hashlib
import importlib.util
import re
import sys
from pathlib import Path

try:
    from jinja2 import Environment, FileSystemLoader
except ImportError:
    print("ERROR: jinja2 required (pip install jinja2)", file=sys.stderr)
    sys.exit(2)

HERE = Path(__file__).resolve().parent
ROLE = HERE.parent
spec = importlib.util.spec_from_file_location("acl_evaluate", ROLE / "files" / "acl-evaluate.py")
ev = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ev)
sys.path.insert(0, str(HERE / "slot-fragment"))
import slotfragment  # noqa: E402

SALT, POOL = "rr3p-test", 6


def anchor(k):
    return hashlib.sha256(f"{SALT}:{k}".encode()).hexdigest()[:32]


def render(assigned, **overrides):
    """assigned: {k: generation} for the slots that are held."""
    env = Environment(loader=FileSystemLoader(str(ROLE / "templates")), trim_blocks=True, lstrip_blocks=True)
    env.filters["hash"] = lambda s, algo="sha256": hashlib.new(algo, str(s).encode()).hexdigest()
    env.filters["bool"] = lambda v: v if isinstance(v, bool) else str(v).strip().lower() in ("true", "yes", "1", "on")
    frag = slotfragment.render([
        (anchor(k), slotfragment.leaf_cn(anchor(k), assigned[k]) if k in assigned else "")
        for k in range(POOL)])
    base = dict(
        goat_zenoh_listen_host="0.0.0.0", goat_zenoh_listen_port=7447, goat_zenoh_rest_port=8000,
        goat_zenoh_retention_days=7, goat_zenoh_audit_retention_days=365,
        goat_zenoh_storage_path="/var/lib/x/rocksdb", goat_zenoh_tls_dir="/etc/x/tls",
        zenoh_seed_vendors=[], zenoh_identity_model="per-uuid-slot-pool",
        zenoh_slot_pool_size=POOL, zenoh_slot_pool_salt=SALT,
        zenoh_universal_read_cns=["panoscope-trial-peer"],
        zenoh_slot_fragment_content=frag,
    )
    base.update(overrides)
    return ev.Acl(env.get_template("config.json5.j2").render(**base))


fails = 0


def check(cond, msg):
    global fails
    print(("  PASS: " if cond else "  FAIL: ") + msg)
    if not cond:
        fails += 1


def writes(acl, cn, k):
    return acl.decide_cn(cn, "put", "ingress", f"{anchor(k)}/telemetry/x") == "allow"


def reads(acl, cn, key):
    return (acl.decide_cn(cn, "declare_subscriber", "ingress", key) == "allow"
            or acl.decide_cn(cn, "put", "egress", key) == "allow")


A, OTHER = 0, 1
a0 = anchor(A)
for label, extra in (
    ("default read (partition off)", {}),
    ("partner partition + scoped read on", dict(
        zenoh_partner_read_partition=True, zenoh_peer_read_scoped=True,
        zenoh_partner_slot_blocks=[{"lanes": ["REL-ALL"], "from": 5, "to": 5}],
        zenoh_partner_read_lanes={}, zenoh_peer_cross_tenant_read=[])),
):
    print(f"== {label} ==")
    probe = f"{anchor(OTHER)}/telemetry/x"
    leaf_a, leaf_b, leaf_b2 = a0 + ".g1", a0 + ".g3", a0 + ".g4"

    s1 = render({A: 1, OTHER: 1}, **extra)
    check(writes(s1, leaf_a, A), "1. holder A's leaf writes its own namespace")
    check(reads(s1, leaf_a, probe), "1. holder A's leaf reads the fabric")
    check(not writes(s1, leaf_a, OTHER), "1. and does not write anyone else's")

    s_free = render({OTHER: 1}, **extra)
    check(not writes(s_free, leaf_a, A) and not reads(s_free, leaf_a, probe),
          "2a. after A's offboard (slot free) A's leaf is refused write and read")

    s2 = render({A: 3, OTHER: 1}, **extra)
    check(not writes(s2, leaf_a, A), "2b. slot reassigned to B: A's leaf cannot write as B")
    check(not reads(s2, leaf_a, probe) and not reads(s2, leaf_a, f"{a0}/telemetry/x"),
          "2b. and cannot read, B's namespace included")
    check(writes(s2, leaf_b, A) and reads(s2, leaf_b, probe), "2b. B's leaf works")

    s3 = render({A: 4, OTHER: 1}, **extra)
    check(not writes(s3, leaf_b, A) and not reads(s3, leaf_b, probe), "3. after B rotates, B's previous leaf is refused")
    check(writes(s3, leaf_b2, A) and reads(s3, leaf_b2, probe), "3. B's new leaf works")

    for k in (2, 3):
        leaf = anchor(k) + ".g1"
        check(not writes(s3, leaf, k) and not reads(s3, leaf, probe), f"4. a leaf for unassigned index {k} is refused")
        check(not writes(s3, anchor(k), k) and not reads(s3, anchor(k), probe),
              f"4. a generation-0 leaf (CN = bare anchor) for unassigned index {k} is refused")
    check(not writes(s3, a0, A) and not reads(s3, a0, probe),
          "5. a pre-change leaf of slot A (CN = bare anchor) is refused once the slot moved on")

    s0 = render({A: 0}, **extra)
    check(writes(s0, a0, A) and reads(s0, a0, probe),
          "migration: a slot still at generation 0 admits its existing bare-anchor leaf")
    check(reads(s3, "panoscope-trial-peer", probe), "a named non-slot reader keeps its read")
    check(s3.decide_cn(leaf_b2, "declare_subscriber", "ingress", "ops/audit/operator/**") == "deny",
          "F-207: a slot's current leaf still cannot watch the operator-audit trail")

print("== co-publish (ADR 1105): each topic's grant follows the registry, not the stale role-rendered rules ==")
T1, T2 = "multipub/pool1/topic1", "multipub/pool1/topic2"


def render_copub(gen, copub, role_grants=None):
    """copub: [(topic, live)] in the fragment for slot A; role_grants: what
    the (possibly stale) role var zenoh_copublish_grants says."""
    env = Environment(loader=FileSystemLoader(str(ROLE / "templates")), trim_blocks=True, lstrip_blocks=True)
    env.filters["hash"] = lambda s, algo="sha256": hashlib.new(algo, str(s).encode()).hexdigest()
    env.filters["bool"] = lambda v: v if isinstance(v, bool) else str(v).strip().lower() in ("true", "yes", "1", "on")
    frag = slotfragment.render([(anchor(k), slotfragment.leaf_cn(anchor(k), gen) if k == A else "", copub if k == A else [])
                                for k in range(POOL)])
    text = env.get_template("config.json5.j2").render(
        goat_zenoh_listen_host="0.0.0.0", goat_zenoh_listen_port=7447, goat_zenoh_rest_port=8000,
        goat_zenoh_retention_days=7, goat_zenoh_audit_retention_days=365,
        goat_zenoh_storage_path="/var/lib/x/rocksdb", goat_zenoh_tls_dir="/etc/x/tls",
        zenoh_seed_vendors=[], zenoh_identity_model="per-uuid-slot-pool",
        zenoh_slot_pool_size=POOL, zenoh_slot_pool_salt=SALT, zenoh_universal_read_cns=[],
        zenoh_copublish_enabled=True, zenoh_multipub_topics=[T1, T2],
        zenoh_copublish_grants=role_grants if role_grants is not None else {a0: [T1, T2]},
        zenoh_slot_fragment_content=frag)
    ids = re.findall(r'^\s*id: "([^"]+)",\n\s*cert_common_names', text, re.M)
    declared = set(ids)
    dupes = {i for i in ids if ids.count(i) > 1}
    used = set(x for l in re.findall(r'rules: \[[^\]]*\],\s*subjects: \[([^\]]*)\]', text)
               for x in re.findall(r'"([^"]+)"', l))
    return ev.Acl(text), (used - declared) | {"duplicate:" + d for d in dupes}


def pub(acl, cn, topic):
    return acl.decide_cn(cn, "put", "ingress", f"{topic}/x") == "allow"


c, undeclared = render_copub(1, [(T1, True)])
check(not undeclared, f"every policy subject is declared (T2 is absent from the fragment: bound to -none): {sorted(undeclared)}")
check(pub(c, a0 + ".g1", T1) and not pub(c, a0 + ".g1", T2),
      "A's current leaf co-publishes T1 (granted) and not T2 (in the role var, no grant)")
c, _ = render_copub(1, [(T1, True), (T2, False)])
check(pub(c, a0 + ".g1", T1) and not pub(c, a0 + ".g1", T2), "two grants, T2 revoked: T1 still works, T2 does not")
c, _ = render_copub(1, [(T1, False)])
check(not pub(c, a0 + ".g1", T1), "an expired (or revoked) grant stops at the next fragment load, the stale rule notwithstanding")
# The reviewer's scenario: A's T1 grant ended (offboard / hand-over), B now
# holds the slot at generation 2 with a grant on T2; the role var is stale
# and still carries A's T1 rule.
c, undeclared = render_copub(2, [(T1, False), (T2, True)])
check(not undeclared, "the stale T1 rule still has a declared subject")
check(pub(c, a0 + ".g2", T2), "B co-publishes T2 (B's own grant)")
check(not pub(c, a0 + ".g2", T1), "B cannot publish on T1 through A's stale rule")
check(not pub(c, a0 + ".g1", T1) and not pub(c, a0 + ".g1", T2), "A's leaf co-publishes nothing")
check(writes(c, a0 + ".g2", A), "B keeps its own-anchor write")
# The reviewer's exact case: the role var still says {a0: [T1]} only (A's
# grant), B holds a grant on T2 only.
c, undeclared = render_copub(3, [(T1, False), (T2, True)], {a0: [T1]})
check(not undeclared and not pub(c, a0 + ".g3", T1),
      "stale role var {A: [T1]}, B granted T2 only: B cannot publish T1")
c, undeclared = render_copub(1, [], {a0: [T1, T1]})
check(not undeclared, "a role var listing the same topic twice renders one -none subject (no duplicate ids)")
c, _ = render_copub(0, [(T1, True)], {a0: [T1]})
check(pub(c, a0, T1), "a pre-upgrade writer (generation 0, bare-anchor CN) with an active grant co-publishes")

print()
print("ALL RR3P CHECKS PASSED" if fails == 0 else f"{fails} CHECK(S) FAILED")
sys.exit(1 if fails else 0)
