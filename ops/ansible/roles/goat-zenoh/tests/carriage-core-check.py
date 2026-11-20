#!/usr/bin/env python3
"""ADR 1251 phase 4: the core router lets a pod write exactly the slots its
carriage rules cover, and an ingress adapter exactly the keyless sources that
name it, and nothing else.

Renders config.json5.j2 with the slot fragment the portal writes (its
carry-<anchor> subjects, slot-fragment/slotfragment.py) and decides each peer's
access with files/acl-evaluate.py (the router's own decision rules, no live
zenohd). Covers the flag off, a pre-carriage fragment (the transition), no
fragment at all (bootstrap), a revoked rule, a topic-budget-narrowed source and
the partner partition.

    python3 ops/ansible/roles/goat-zenoh/tests/carriage-core-check.py
"""
import hashlib
import importlib.util
import re
import sys
from pathlib import Path

try:
    from jinja2 import Environment, FileSystemLoader, select_autoescape
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

SALT, POOL = "carriage-test", 6
POD, SRC, OTHER, ADAPTER, KEYLESS = 0, 1, 2, 3, 4


def anchor(k):
    return hashlib.sha256(f"{SALT}:{k}".encode()).hexdigest()[:32]


GEN = {POD: 2, SRC: 1, OTHER: 1, ADAPTER: 1, KEYLESS: 1}
CN = {k: slotfragment.leaf_cn(anchor(k), g) for k, g in GEN.items()}


def fragment(carry, with_carry=True):
    """carry: {k: [carrier k, ...]}. with_carry=False renders a fragment the
    way a portal before phase 4 did (no carry- subjects)."""
    text = slotfragment.render([
        (anchor(k), CN.get(k, ""), [], sorted(CN[c] for c in carry.get(k, [])))
        for k in range(POOL)])
    if not with_carry:
        text = re.sub(r'      \{\n        id: "carry-[^"]+",\n        cert_common_names: [^\n]+\n      \},\n', "", text)
    return text


def render(frag, **overrides):
    # The template renders JSON5, not HTML: autoescape stays on only for .html.
    env = Environment(loader=FileSystemLoader(str(ROLE / "templates")), trim_blocks=True, lstrip_blocks=True,
                      autoescape=select_autoescape(enabled_extensions=("html",), default_for_string=False))
    env.filters["hash"] = lambda s, algo="sha256": hashlib.new(algo, str(s).encode()).hexdigest()
    env.filters["bool"] = lambda v: v if isinstance(v, bool) else str(v).strip().lower() in ("true", "yes", "1", "on")
    base = dict(
        goat_zenoh_listen_host="0.0.0.0", goat_zenoh_listen_port=7447, goat_zenoh_rest_port=8000,
        goat_zenoh_retention_days=7, goat_zenoh_audit_retention_days=365,
        goat_zenoh_storage_path="/var/lib/x/rocksdb", goat_zenoh_tls_dir="/etc/x/tls",
        zenoh_seed_vendors=[], zenoh_identity_model="per-uuid-slot-pool",
        zenoh_slot_pool_size=POOL, zenoh_slot_pool_salt=SALT,
        zenoh_universal_read_cns=["panoscope-trial-peer"],
        zenoh_slot_fragment_content=frag,
        zenoh_carriage_core_enabled=True,
    )
    base.update(overrides)
    return env.get_template("config.json5.j2").render(**base)


def subjects_ok(text):
    """Every subject a policy names is declared, and no id is declared twice."""
    ids = re.findall(r'^\s*id: "([^"]+)",\n\s*cert_common_names', text, re.M)
    used = set(x for l in re.findall(r'rules: \[[^\]]*\],\s*subjects: \[([^\]]*)\]', text)
               for x in re.findall(r'"([^"]+)"', l))
    problems = (used - set(ids)) | {"duplicate:" + i for i in ids if ids.count(i) > 1}
    return sorted(problems)


fails = 0


def check(cond, msg):
    global fails
    print(("  PASS: " if cond else "  FAIL: ") + msg)
    if not cond:
        fails += 1


def writes(acl, cn, key):
    return acl.decide_cn(cn, "put", "ingress", key) == "allow"


def reads(acl, cn, key):
    return acl.decide_cn(cn, "declare_subscriber", "ingress", key) == "allow"


S, O, K, P = anchor(SRC), anchor(OTHER), anchor(KEYLESS), anchor(POD)
RULES = {SRC: [POD], KEYLESS: [ADAPTER, POD]}

for label, extra in (
    ("default read (partition off)", {}),
    ("partner partition + scoped read on", dict(
        zenoh_partner_read_partition=True, zenoh_peer_read_scoped=True,
        zenoh_partner_slot_blocks=[{"lanes": ["REL-ALL"], "from": 5, "to": 5}],
        zenoh_partner_read_lanes={}, zenoh_peer_cross_tenant_read=[])),
):
    print(f"== carriage on, {label} ==")
    text = render(fragment(RULES), **extra)
    acl = ev.Acl(text)
    check(not subjects_ok(text), f"every policy subject declared once: {subjects_ok(text)}")
    for key in (f"{S}/telemetry/x", f"volatile/{S}/x", f"{S}/x/@adv/y"):
        check(writes(acl, CN[POD], key), f"the pod writes the carried source's {key.replace(S, '<src>')}")
    check(not writes(acl, CN[POD], f"{O}/telemetry/x"), "the pod does not write a source no rule covers")
    check(writes(acl, CN[POD], f"{P}/telemetry/x"), "the pod keeps its own-slot write")
    check(writes(acl, CN[SRC], f"{S}/telemetry/x"), "the source's own leaf still writes its slot")
    check(writes(acl, CN[ADAPTER], f"{K}/stream/asterix/48/raw/v1"), "the adapter writes the keyless source that names it")
    check(not writes(acl, CN[ADAPTER], f"{S}/telemetry/x"), "the adapter does not write another source")
    check(writes(acl, CN[POD], f"{K}/stream/asterix/48/raw/v1"), "a pod covering the keyless source writes it too")
    check(not writes(acl, CN[POD], "ops/audit/operator/x") and not reads(acl, CN[POD], "ops/audit/operator/**"),
          "carriage gives the pod nothing on the operator-audit trail")
    base = ev.Acl(render(fragment({}), **extra))
    for key in (f"{O}/telemetry/x", "ops/names/claim/x", "**"):
        check(reads(acl, CN[POD], key) == reads(base, CN[POD], key),
              f"carriage changes no read of the pod ({key.replace(O, '<other>')})")
    old_pod = slotfragment.leaf_cn(P, 1)
    check(not writes(acl, old_pod, f"{S}/telemetry/x"), "the pod's previous leaf (generation 1) carries nothing")

print("== a rule revoked (the portal re-renders the fragment without the carrier) ==")
acl = ev.Acl(render(fragment({KEYLESS: [ADAPTER]})))
check(not writes(acl, CN[POD], f"{S}/telemetry/x"), "the pod no longer writes the source")
check(writes(acl, CN[ADAPTER], f"{K}/x"), "the adapter keeps the keyless source")

print("== a source narrowed by a topic-budget grant ==")
acl = ev.Acl(render(fragment(RULES), zenoh_topic_budget_grants={S: {"prefixes": ["video"]}}))
check(writes(acl, CN[POD], f"{S}/video/x"), "the pod writes the source's granted prefix")
check(not writes(acl, CN[POD], f"{S}/other/x"), "and nothing the source itself may not write")

print("== transition: a fragment rendered before the portal knew carriage ==")
text = render(fragment(RULES, with_carry=False))
acl = ev.Acl(text)
check(not subjects_ok(text), f"every policy subject declared once: {subjects_ok(text)}")
check(f'id: "carry-{S}-none"' in text, "the carriage policy binds to an inert -none subject")
check(not writes(acl, CN[POD], f"{S}/telemetry/x"), "nobody carries anything until the portal renders carry- subjects")

print("== bootstrap: no fragment on the host ==")
text = render("")
acl = ev.Acl(text)
check(not subjects_ok(text), f"every policy subject declared once: {subjects_ok(text)}")
check(f'id: "carry-{S}",\n        cert_common_names: ["{S}.unassigned"],' in text, "carry- subjects declared with the unassigned CN")
check(not writes(acl, CN[POD], f"{S}/telemetry/x"), "nobody carries anything")

print("== the portal splices a carriage fragment into a config the role rendered with one ==")
first = render(fragment({}))
spliced = re.sub(r"(      // BEGIN: slot subjects\n).*?(      // END: slot subjects)",
                 lambda m: m.group(1) + fragment(RULES) + m.group(2), first, flags=re.S)
acl = ev.Acl(spliced)
check(not subjects_ok(spliced), f"every policy subject declared once after the splice: {subjects_ok(spliced)}")
check(writes(acl, CN[POD], f"{S}/telemetry/x"), "the new carrier is admitted with no role run")

print("== carriage off (the default) ==")
text = render(fragment(RULES), zenoh_carriage_core_enabled=False)
acl = ev.Acl(text)
check(not subjects_ok(text), f"every policy subject declared once: {subjects_ok(text)}")
check(not re.search(r'subjects: \["carry-', text), "no policy names a carry- subject")
check(not writes(acl, CN[POD], f"{S}/telemetry/x"), "the pod writes nothing for the source")
off = render(fragment({}), zenoh_carriage_core_enabled=False)
check("carry-" not in re.sub(r"// BEGIN: slot subjects.*?// END: slot subjects", "", off, flags=re.S),
      "outside the portal's fragment the render names no carry- subject")
nofrag_off = render("", zenoh_carriage_core_enabled=False)
check("carry-" not in nofrag_off, "with no fragment and the flag off, the render names no carry- subject at all")

print()
print("ALL CARRIAGE CORE CHECKS PASSED" if fails == 0 else f"{fails} CHECK(S) FAILED")
sys.exit(1 if fails else 0)
