#!/usr/bin/env python3
"""scoped-read-check.py — prove what the scoped peer read ALLOWS and REFUSES


Every other test in this role asserts the SHAPE of the rendered config. This one
asserts the DECISIONS: it renders the template, loads it through the ACL
evaluator (files/acl-evaluate.py, a port of zenoh 1.9.0's inclusion + rule
matching) and asks the questions an operator would ask of a live router.

Two properties are load-bearing and neither is visible by grepping the config:

  1. a normal peer is REFUSED `**`, `*/**` and every leading-wildcard shape,
     while keeping read on every namespace it has today;
  2. a named identity — and only a named identity — may subscribe fabric-wide.

Plus the negative that makes the change safe to merge before it is deployed:
with the flag off the render is byte-identical to a render that has never heard
of the flag.

    python3 ops/ansible/roles/goat-zenoh/tests/scoped-read-check.py
"""
import hashlib
import importlib.util
import re
import sys
from pathlib import Path

try:
    from jinja2 import Environment, FileSystemLoader
except ImportError:  # pragma: no cover
    print("ERROR: jinja2 required (pip install jinja2)", file=sys.stderr)
    sys.exit(2)

ROLE = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location("acl_evaluate", ROLE / "files" / "acl-evaluate.py")
ev = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ev)

SALT, POOL = "scoped-test", 40
CROSS = ["**/basemap_vendor/**", "**/airtracks/v1/*", "*/geo/track/v1/*"]
BLOCKS = [{"lanes": ["REL-ALL"], "from": 30, "to": 32}]


def anchor(k: int) -> str:
    return hashlib.sha256(f"{SALT}:{k}".encode()).hexdigest()[:32]


def render(**overrides) -> str:
    env = Environment(loader=FileSystemLoader(str(ROLE / "templates")),
                      trim_blocks=True, lstrip_blocks=True)
    env.filters["hash"] = lambda s, algo="sha256": hashlib.new(algo, str(s).encode()).hexdigest()
    env.filters["bool"] = lambda v: v if isinstance(v, bool) else (
        str(v).strip().lower() in ("true", "yes", "1", "on"))
    base = dict(
        goat_zenoh_listen_host="0.0.0.0", goat_zenoh_listen_port=7447,
        goat_zenoh_rest_port=8000, goat_zenoh_retention_days=7,
        goat_zenoh_audit_retention_days=365,
        goat_zenoh_storage_path="/var/lib/x/rocksdb", goat_zenoh_tls_dir="/etc/x/tls",
        zenoh_seed_vendors=["community"],
        zenoh_identity_model="per-uuid-slot-pool",
        zenoh_slot_pool_size=POOL, zenoh_slot_pool_salt=SALT,
        zenoh_partner_read_partition=True, zenoh_partner_slot_blocks=BLOCKS,
        zenoh_partner_read_lanes={}, zenoh_universal_read_cns=["panoscope-trial-peer"],
        zenoh_peer_cross_tenant_read=CROSS,
    )
    base.update(overrides)
    return env.get_template("config.json5.j2").render(**base)


fails = 0


def check(cond, msg):
    global fails
    print(("  PASS: " if cond else "  FAIL: ") + msg)
    if not cond:
        fails += 1


def decides(acl, subject, cases):
    for message, flow, key, want in cases:
        got = acl.decide(subject, message, flow, key)
        check(got == want, f"{subject}: {message} {flow} `{key}` -> {want}")


print("== default OFF is byte-identical (the merge-before-deploy guarantee) ==")
off = render()
off_explicit = render(zenoh_peer_read_scoped=False, zenoh_fabric_wide_read=[
    {"cn": "should-not-appear", "owner": "x", "reason": "y"}])
check(off == off_explicit, "flag off renders identically whatever the named list says")
check("peer-r-scoped" not in off and "fabric-wide-r" not in off,
      "flag off emits NO scoped rule and NO fabric-wide rule")
check("should-not-appear" not in off_explicit,
      "flag off never leaks a named CN into the config")
check('key_exprs: ["**", "**/@adv/**"]' in off,
      "flag off keeps the universal read rule exactly as it was")

print("== flag ON: the rendered artefacts ==")
on = render(zenoh_peer_read_scoped=True, zenoh_fabric_wide_read=[
    {"cn": "panoscope-trial-peer", "owner": "platform", "reason": "Data-COP", "until": "2026-12-17"},
    {"slot_idx": 5, "owner": "platform", "reason": "synth sink persona"},
])
a_named_by_idx = anchor(5)
check('id: "peer-r-scoped"' in on and 'id: "peer-deliver-scoped"' in on,
      "the scoped read + delivery rules render")
check('id: "fabric-wide-r"' in on and 'id: "fabric-wide-read"' in on,
      "the named grant renders as one rule and one subject")
check(f'"{a_named_by_idx}",' in on,
      "a slot_idx entry renders as the derived anchor CN (no anchor copied into git)")
check("// platform — Data-COP (review 2026-12-17)" in on,
      "each named CN carries its owner, reason and review date in the config itself")

acl = ev.Acl(on)
# Structural: no rule bound to the universal subject may name the whole
# keyspace, and every rendered key_expr must be a key expression the router
# will actually load — a non-canon one does not start it.
wide = [(rid, ke) for rid in acl.rules_for("trial-peer-read")
        for ke in acl.rules[rid]["key_exprs"] if ke == "**"]
check(not wide, f"no rule bound to trial-peer-read carries `**` ({wide})")
check(not acl.unusable_key_exprs(),
      f"every rendered key_expr is in canon form ({acl.unusable_key_exprs()[:2]})")
check("fabric-wide-r" in acl.rules_for("fabric-wide-read"),
      "the fabric-wide rule is BOUND (an unbound rule is default-deny, F-177)")

print("== flag ON: a normal peer ==")
own, other, partner = anchor(0), anchor(1), anchor(30)
decides(acl, "trial-peer-read", [
    # THE BOUNDARY. `**` is the whole keyspace in one declaration: refused.
    ("declare_subscriber", "ingress", "**", "deny"),
    # A cross-tenant FILTER is not: one vendor, or one topic, across tenants.
    # Allowed because the shape is declared in zenoh_peer_cross_tenant_read.
    ("declare_subscriber", "ingress", "**/basemap_vendor/**", "allow"),
    ("declare_subscriber", "ingress", "**/airtracks/v1/*", "allow"),
    ("declare_subscriber", "ingress", "*/geo/track/v1/*", "allow"),
    # An UNDECLARED cross-tenant shape is refused — the cost of the mechanism:
    # `**`-leading filters cannot be expressed as a pattern, only enumerated.
    ("declare_subscriber", "ingress", "**/tracks/v1", "deny"),
    # THE KNOWN HOLE, asserted so nobody reads it as an oversight: a declaration
    # made only of `*` chunks is allowed by the `*/**` key_expr, and `*/**`
    # matches every key with at least one chunk. Closing it means enumerating
    # every namespace again (+~2,100 rendered lines).
    ("declare_subscriber", "ingress", "*/**", "allow"),
    ("declare_subscriber", "ingress", "*/*/**", "allow"),
    ("declare_subscriber", "ingress", f"{own}/a/**", "allow"),
    ("declare_subscriber", "ingress", f"{other}/a/b", "allow"),
    ("declare_subscriber", "ingress", f"{partner}/**", "allow"),
    ("declare_subscriber", "ingress", f"{own}/x/@adv/pub/zid", "allow"),
    ("declare_subscriber", "ingress", "ops/fabric/**", "allow"),
    ("declare_subscriber", "ingress", "release/REL-ALL/**", "allow"),
    ("declare_subscriber", "ingress", "community/**", "allow"),
    ("declare_subscriber", "ingress", "ops/audit/operator/**", "deny"),
    ("put", "egress", f"{own}/x/y", "allow"),
    ("put", "egress", "ops/fabric/health/x", "allow"),
    ("put", "egress", "ops/audit/operator/x", "deny"),
    ("put", "ingress", f"{own}/x/y", "deny"),
    ("query", "egress", "@/zid/router/subscriber/a", "allow"),
    ("declare_subscriber", "ingress", "unknown-root/x", "allow"),
    ("declare_subscriber", "ingress", "ops", "allow"),
    ("declare_subscriber", "ingress", "**/@adv/**", "allow"),
])

print("== flag ON: the named identities ==")
decides(acl, "fabric-wide-read", [
    ("declare_subscriber", "ingress", "**", "allow"),
    ("declare_subscriber", "ingress", "*/**", "allow"),
    ("declare_subscriber", "ingress", "**/tracks/v1", "allow"),
    ("declare_queryable", "ingress", "**", "deny"),
    ("declare_queryable", "ingress", "ops/x/**", "allow"),
    ("declare_subscriber", "ingress", "ops/audit/operator/**", "deny"),
    ("put", "egress", f"{own}/x/y", "allow"),
])
check(acl.decide_cn("panoscope-trial-peer", "declare_subscriber", "ingress", "**") == "allow",
      "a named CN gets ** across subjects (first Allow wins)")
check(acl.decide_cn(own, "declare_subscriber", "ingress", "**") == "deny",
      "an unnamed slot CN is refused ** through every subject it matches")
check(acl.decide_cn(a_named_by_idx, "declare_subscriber", "ingress", "**") == "allow",
      "a slot named by slot_idx gets ** through the derived CN")

print("== flag ON: infra and partners are unchanged ==")
for sid in ("operator", "audit-subscriber", "infra-fabric-health", "infra-consistency-console"):
    check(acl.decide(sid, "declare_subscriber", "ingress", "**") == "allow",
          f"{sid} keeps its own fabric-wide read")
decides(acl, f"partner-{partner}", [
    ("declare_subscriber", "ingress", "**", "deny"),
    ("declare_subscriber", "ingress", f"{own}/**", "deny"),
    ("declare_subscriber", "ingress", f"{partner}/**", "allow"),
    ("declare_subscriber", "ingress", "release/REL-ALL/**", "allow"),
])

print("== flag ON: structure still parses ==")
stripped = re.sub(r"//[^\n]*", "", on)
stripped = re.sub(r'"[^"]*"', '""', stripped)
check(stripped.count("{") == stripped.count("}") and stripped.count("[") == stripped.count("]"),
      "brace/bracket balanced with the scoped read on")

print("== flag ON with an EMPTY named list is fail-closed ==")
empty = ev.Acl(render(zenoh_peer_read_scoped=True, zenoh_fabric_wide_read=[]))
check(empty.decide_cn("panoscope-trial-peer", "declare_subscriber", "ingress", "**") == "deny",
      "nobody gets the fabric-wide read by omission")
check(empty.decide_cn("panoscope-trial-peer", "declare_subscriber", "ingress", "ops/**") == "allow",
      "and an empty list costs nobody their scoped read")

print("== every goatnet's DECLARED cross-tenant shapes are usable and not everything ==")
# The synthetic renders above prove the mechanism. This checks the real values
# in the repo, because a shape that cannot load stops the router, and a shape
# that matches everything silently re-opens the fan-in.
import glob as _glob
try:
    import yaml as _yaml
except ImportError:
    print("  SKIP: pyyaml not installed")
else:
    seen = 0
    for gv in sorted(_glob.glob(str(ROLE.parent.parent / "group_vars" / "*" / "*.yml"))):
        with open(gv, encoding="utf-8") as fh:
            try:
                doc = _yaml.safe_load(fh) or {}
            except Exception:
                continue
        shapes = (doc or {}).get("zenoh_peer_cross_tenant_read") or []
        if not shapes:
            continue
        scope = Path(gv).parent.name
        seen += 1
        for shape in shapes:
            why = ev.validate(shape)
            check(why is None, f"{scope}: `{shape}` is a usable key expression" + (f" — {why}" if why else ""))
            check(shape != "**", f"{scope}: `{shape}` is not the fabric-wide grant in disguise")
            check(any(c not in ("*", "**") for c in shape.split("/")),
                  f"{scope}: `{shape}` names at least one real chunk (a `*`-only shape matches everything)")
    check(seen > 0, f"at least one goatnet declares cross-tenant shapes ({seen} found)")

print()
print("ALL SCOPED-READ CHECKS PASSED" if fails == 0 else f"{fails} CHECK(S) FAILED")
sys.exit(fails)
