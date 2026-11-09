#!/usr/bin/env python3
"""Render-validate config.json5.j2 in both identity models (ADR 1025).

Proves (a) per-vendor mode is unchanged — nova-a regression guard — and
(b) per-uuid-slot-pool mode emits one isolated slot per k with the
deterministic anchor sha256(salt:k)[:32] as both the cert CN and the
write anchor. Pure Jinja render check (no running router); the live
isolation proof is the separate isolated-router harness.

    python3 ops/ansible/roles/goat-zenoh/tests/render-check.py
"""
import hashlib
import re
import sys
from pathlib import Path

try:
    from jinja2 import Environment, FileSystemLoader
except ImportError:
    print("ERROR: jinja2 required (pip install jinja2)", file=sys.stderr)
    sys.exit(2)

TPL_DIR = Path(__file__).resolve().parent.parent / "templates"
TPL = "config.json5.j2"

# The SAME formula the router template + the portal must agree on.
def anchor(salt: str, k: int) -> str:
    return hashlib.sha256(f"{salt}:{k}".encode()).hexdigest()[:32]


def render(**overrides) -> str:
    # trim_blocks + lstrip_blocks match Ansible's template module so this
    # render matches what the role actually produces.
    env = Environment(loader=FileSystemLoader(str(TPL_DIR)), trim_blocks=True, lstrip_blocks=True)
    # Match Ansible's `hash('sha256')` (hexdigest of the utf-8 string).
    env.filters["hash"] = lambda s, algo="sha256": hashlib.new(algo, str(s).encode()).hexdigest()
    # Match Ansible's `bool` filter (used to gate the transitional system/**
    # namespace, ADR 1035 / #922). Vanilla Jinja2 has no `bool` filter.
    env.filters["bool"] = lambda v: v if isinstance(v, bool) else (
        str(v).strip().lower() in ("true", "yes", "1", "on"))
    base = dict(
        goat_zenoh_listen_host="0.0.0.0",
        goat_zenoh_listen_port=7447,
        goat_zenoh_rest_port=8000,
        goat_zenoh_retention_days=7,
        goat_zenoh_audit_retention_days=365,
        goat_zenoh_storage_path="/var/lib/zagros-zenoh/rocksdb",
        goat_zenoh_tls_dir="/etc/zagros-zenoh/tls",
        zenoh_seed_vendors=["community", "caracal"],
        zenoh_identity_model="per-vendor",
        zenoh_slot_pool_size=256,
        zenoh_slot_pool_salt="",
        # Transmit-queue policy - role defaults restated, because this
        # dict is what the template renders against. queue_size is None by
        # default: measured to buy nothing, and the expensive knob.
        goat_zenoh_tx_queue_size=None,
        goat_zenoh_tx_block_wait_before_close_us=2000000,
    )
    base.update(overrides)
    return env.get_template(TPL).render(**base)


def brace_balanced(s: str) -> bool:
    # crude structural sanity: strip // comments + strings, count braces
    s = re.sub(r"//[^\n]*", "", s)
    s = re.sub(r'"[^"]*"', '""', s)
    return s.count("{") == s.count("}") and s.count("[") == s.count("]")


fails = 0
def check(cond, msg):
    global fails
    print(("  PASS: " if cond else "  FAIL: ") + msg)
    if not cond:
        fails += 1


print("== per-vendor mode (nova-a — must be unchanged) ==")
pv = render(zenoh_identity_model="per-vendor")
# Since the F-258 seed/org split, per-vendor mode renders EMPTY per-vendor
# marker regions (the portal fills them at runtime) rather than inline seed
# vendor stanzas — assert the markers, not removed community/caracal stanzas.
check('// BEGIN: per-vendor write rules' in pv and '// BEGIN: per-vendor subjects' in pv,
      "per-vendor mode renders the empty per-vendor write-rule + subject marker regions")
check('// BEGIN: per-vendor policies' in pv and '// BEGIN: per-vendor storages' in pv,
      "per-vendor mode renders the empty per-vendor policy + storage marker regions")
check('id: "slot-w-' not in pv and 'id: "slot-' not in pv, "no slot-* rule/subject ids in per-vendor mode")
check(brace_balanced(pv), "brace/bracket balanced")

print("== transmit-queue policy ==")
# A var this dict forgets renders as the empty string, which still balances
# braces and still passes every check above - `control: ,` reaches the router
# and zenohd refuses to start. Assert the values, and assert that NO key in the
# whole config rendered empty.
check("wait_before_close: 2000000," in pv, "block wait_before_close renders its configured value")
check("size: {" not in pv, "tx queue size is ABSENT by default (zenoh's own default of 2 applies)")
q8 = render(goat_zenoh_tx_queue_size=8)
check(q8.count("data_low: 8,") == 1 and "control: 8," in q8,
      "a goatnet that sets the queue size gets it rendered on every priority")
empty = re.findall(r"^\s*([a-z_]+):\s*(?:,|\}|$)", pv, re.M)
check(not empty, f"no config key rendered empty (would be an undefined var): {empty[:5]}")

print("== per-uuid-slot-pool mode (ZAGROS) ==")
SALT, N = "zagros-test", 4
sp = render(zenoh_identity_model="per-uuid-slot-pool", zenoh_slot_pool_salt=SALT, zenoh_slot_pool_size=N)
anchors = [anchor(SALT, k) for k in range(N)]
check(len(set(anchors)) == N, f"{N} distinct deterministic anchors")
for k, a in enumerate(anchors):
    check(f'"slot-{a}": {{' in sp and f'key_expr: "{a}/**"' in sp, f"slot {k}: storage anchor {a[:8]}…/**")
    check(f'id: "slot-w-{a}"' in sp and f'key_exprs: ["{a}/**", "volatile/{a}/**", "{a}/**/@adv/**"]' in sp, f"slot {k}: write rule grants {a[:8]}…/** only")
    check(f'id: "slot-{a}"' in sp and f'cert_common_names: ["{a}"]' in sp, f"slot {k}: subject CN = anchor {a[:8]}…")
    check(f'rules: ["slot-w-{a}", "trial-peer-w-attest-schema"]' in sp and f'subjects: ["slot-{a}"]' in sp, f"slot {k}: policy binds subject→its own rule")
# isolation property: slot 0's rule must NOT grant slot 1's anchor
check(f'"slot-w-{anchors[0]}"' in sp and anchors[1] not in sp.split(f'slot-w-{anchors[0]}')[1].split("}")[0],
      "slot 0's write rule does not mention any other anchor (isolation)")
# F-517: a SEEDED vendor (zenoh_seed_vendors) publishes with
# CN=<vendor>-trial-peer even on a slot-pool goatnet (airdefenders via the
# rest-pub-bridge sidecar). Previously slot-pool mode rendered nothing for
# it, so the CN matched only trial-peer-read and every PUT was dropped silently
# at ingress. Seeded vendors get the same write rule + subject + policy
# the per-vendor org fragment gives them; nothing else per-vendor appears.
for v in ("community", "caracal"):
    check(f'id: "trial-peer-w-{v}"' in sp and f'key_exprs: ["{v}/**", "volatile/{v}/**", "{v}/**/@adv/**"]' in sp,
          f"seeded vendor {v}: write rule grants {v}/** only")
    check(f'id: "vendor-{v}"' in sp and f'cert_common_names: ["{v}-trial-peer"]' in sp,
          f"seeded vendor {v}: subject CN = {v}-trial-peer")
    check(f'rules: ["trial-peer-w-{v}", "trial-peer-w-attest-schema"]' in sp and f'subjects: ["vendor-{v}"]' in sp,
          f"seeded vendor {v}: policy binds subject→its own rule")
sp_noseed = render(zenoh_identity_model="per-uuid-slot-pool", zenoh_slot_pool_salt=SALT, zenoh_slot_pool_size=N, zenoh_seed_vendors=[])
check("trial-peer-w-community" not in sp_noseed and "vendor-community" not in sp_noseed,
      "no seeded vendors ⇒ no per-vendor entries in slot-pool mode")
check(brace_balanced(sp_noseed), "brace/bracket balanced with no seeded vendors")
check(brace_balanced(sp), "brace/bracket balanced")

# F-207 operator-action-audit restricted-read carve-out — present in BOTH modes
# (not inside the per-vendor/slot-pool conditional). Vendors/slots must be
# denied reads on ops/audit/operator/**; operator + audit-subscriber rescued.
print("== operator-action-audit carve-out ==")
for label, rendered in (("per-vendor", pv), ("slot-pool", sp)):
    check('id: "audit-operator-read-deny"' in rendered
          and 'key_exprs: ["ops/audit/operator/**"]' in rendered
          and 'permission: "deny"' in rendered,
          f"{label}: audit-operator-read-deny rule present + scoped to ops/audit/operator/**")
    tpr_policies = [l for l in rendered.splitlines() if 'subjects: ["trial-peer-read"]' in l]
    check(any('"audit-operator-read-deny"' in l for l in tpr_policies),
          f"{label}: deny bound in the universal trial-peer-read policy (order-independent)")
    check('id: "audit-subscriber"' in rendered
          and 'cert_common_names: ["audit-subscriber", "audit-subscriber-trial-peer"]' in rendered,
          f"{label}: audit-subscriber subject present (CN=audit-subscriber[-trial-peer])")
    check('subjects: ["audit-subscriber"]' in rendered,
          f"{label}: audit-subscriber policy present (rescues persistence past the deny)")

# ADR 1105 multi-publisher topics — additive, default-off, roots inviolable.
print("== co-publish + multi-publisher topics (ADR 1105, slot-pool) ==")
# Default (flag off / nothing registered): the slot-pool render emits NOT ONE
# copub rule or multipub-topic store, so the isolation proof above is untouched
# and the default is byte-identical.
check("copub-w-" not in sp, "default slot-pool render emits NO co-publish rule (opt-in; default byte-identical)")
check('"st-multipub' not in sp and "multipub/" not in sp, "default slot-pool render has NO multipub-topic namespace at all")
# With registered multi-publisher topics + a publisher grant: publisher
# anchors[0] may publish two multi-publisher topics, both under org O's stable
# pool handle.
w = anchors[0]
pool = "1a2b3c4d"                       # a stable opaque org-pool handle
st1 = f"multipub/{pool}/aaaa1111"
st2 = f"multipub/{pool}/bbbb2222"
cp = render(zenoh_identity_model="per-uuid-slot-pool", zenoh_slot_pool_salt=SALT, zenoh_slot_pool_size=N,
            zenoh_copublish_enabled=True,
            zenoh_multipub_topics=[st1, st2],
            zenoh_copublish_grants={w: [st1, st2]})
# The GUARDRAIL, asserted structurally: every copub target is a multipub/ key,
# and NO single-owner anchor ever appears as a copub target.
check(f'key_exprs: ["{st1}/**", "volatile/{st1}/**", "{st1}/**/@adv/**"]' in cp,
      "co-publish rule targets the multipub/<pool>/<topic> namespace (+ volatile + @adv)")
check(f'id: "copub-w-{w}-0"' in cp and f'id: "copub-w-{w}-1"' in cp,
      "one copub rule per granted multi-publisher topic")
# The rule ID names the PUBLISHER (copub-w-<publisher>-<n>) — legitimate. What
# must NEVER appear is a single-owner <uuid>/** root as a copub rule's key-expr
# TARGET. Scan each copub rule body and assert no anchor's own root is granted.
copub_bodies = cp.split('id: "copub-w-')[1:]
no_root_target = all(f'["{a}/**", "volatile/{a}/**"' not in seg
                     for seg in copub_bodies for a in anchors)
check(len(copub_bodies) > 0 and no_root_target,
      "NO single-owner <uuid>/** root is ever a co-publish target (roots inviolable)")
check(f'rules: ["copub-w-{w}-0", "copub-w-{w}-1"],' in cp,
      "co-publish policy binds BOTH the publisher's copub rules")
check(cp.count(f'subjects: ["slot-{w}"]') >= 2,
      "co-publish policy binds onto the publisher's EXISTING slot subject (2nd policy entry)")
check(f'id: "slot-w-{w}"' in cp and f'key_exprs: ["{w}/**", "volatile/{w}/**", "{w}/**/@adv/**"]' in cp,
      "publisher's own slot-w rule is preserved unchanged (additive)")
# The multi-publisher topics get their own rocksdb stores, distinct from every
# slot store.
check(f'"st-multipub-{pool}-aaaa1111"' in cp and f'key_expr: "{st1}/**"' in cp,
      "multi-publisher topic st1 gets its own rocksdb store on multipub/<pool>/<topic>/**")
check(f'"st-multipub-{pool}-bbbb2222"' in cp and f'key_expr: "{st2}/**"' in cp,
      "multi-publisher topic st2 gets its own rocksdb store")
check(cp.count('id: "copub-w-') == 2, "exactly one copub rule per granted multi-publisher topic (no over-generation)")
check(brace_balanced(cp), "brace/bracket balanced with multi-publisher topics + co-publish grants")

# Release-lane WRITE grants — the producer half of partner sharing
# (partner-data-exchange.md / ADR 0821 + ADR 1027). WRITE counterpart to the
# B2.1 partner READ lanes: additive, registry-driven, default-off, roots inviolable.
print("== release-lane write grants (partner-data-exchange, slot-pool) ==")
# Default (nothing registered): NO release-w rule — default byte-identical.
check("release-w-" not in sp, "default slot-pool render emits NO release-writer rule (opt-in; default byte-identical)")
# Producer anchors[0] authorized to publish into two release sets.
prod = anchors[0]
rw = render(zenoh_identity_model="per-uuid-slot-pool", zenoh_slot_pool_salt=SALT, zenoh_slot_pool_size=N,
            zenoh_release_writers={prod: ["ZAGROS", "COALITION"]})
check(f'id: "release-w-{prod}"' in rw, "producer gets a release-w-<anchor> rule")
check(f'"release/ZAGROS/**"' in rw and f'"release/COALITION/**"' in rw,
      "release-w rule grants put/delete on release/<set>/** for each granted set")
# The GUARDRAIL, asserted structurally: every release-w target is a release/ key,
# and NO single-owner <uuid>/** root ever appears as a release-w key-expr target.
rw_bodies = rw.split('id: "release-w-')[1:]
rw_body = rw_bodies[0].split("},")[0] if rw_bodies else ""
check(len(rw_bodies) == 1, "exactly one release-w rule per producer (no over-generation)")
check(all(f'"{a}/**"' not in rw_body for a in anchors),
      "NO single-owner <uuid>/** root is ever a release-write target (roots inviolable)")
check(all(seg.startswith("release/") for seg in re.findall(r'"([^"]+)/\*\*"', rw_body)),
      "every release-w key-expr is under the release/ prefix — nothing else")
check(f'rules: ["release-w-{prod}"]' in rw and rw.count(f'subjects: ["slot-{prod}"]') >= 2,
      "release-writer policy binds onto the producer's EXISTING slot subject (2nd policy entry)")
check(f'id: "slot-w-{prod}"' in rw and f'key_exprs: ["{prod}/**", "volatile/{prod}/**", "{prod}/**/@adv/**"]' in rw,
      "producer's own slot-w rule is preserved unchanged (additive)")
check(brace_balanced(rw), "brace/bracket balanced with release-writer grants")

print()
print("== edge-queryable lockdown + its re-grants ==")
# Default OFF must be byte-identical to today: the universal rule keeps
# declare_queryable and no lockdown artefact appears anywhere.
check("lockdown-q-" not in sp, "default render emits NO lockdown re-grant (opt-in)")
check('"declare_queryable"' in sp.split('id: "trial-peer-r-all"')[1].split("},")[0],
      "with the flag OFF, trial-peer-r-all still carries declare_queryable")

# Flag ON: the universal rule loses the right to answer, and keeps it only on
# the AdvancedPublisher admin space.
lk = render(zenoh_identity_model="per-uuid-slot-pool", zenoh_slot_pool_salt=SALT, zenoh_slot_pool_size=N,
            zenoh_edge_queryable_lockdown=True,
            zenoh_edge_queryable_lockdown_grants=[
                {"subject": "infra-keyd", "key_exprs": ["keys/**"]},
                {"subject": "operator", "key_exprs": ["ops/**"]},
            ])
lk_universal = lk.split('id: "trial-peer-r-all"')[1].split("},")[0]
check('"declare_queryable"' not in lk_universal,
      "with the flag ON, trial-peer-r-all LOSES declare_queryable (the whole point)")
check('id: "trial-peer-adv-queryable"' in lk and '"**/@adv/**"' in lk,
      "the @adv recovery path keeps its own declare_queryable grant")

# The re-grants: one rule per entry, scoped to that subject's OWN namespace.
check('id: "lockdown-q-infra-keyd"' in lk and 'id: "lockdown-q-operator"' in lk,
      "each declared subject gets its own lockdown-q rule")
keyd_body = lk.split('id: "lockdown-q-infra-keyd"')[1].split("},")[0]
check('"keys/**"' in keyd_body and '"ops/**"' not in keyd_body,
      "a re-grant is scoped to its OWN namespace, not another service's")
check('"**"' not in re.findall(r'key_exprs: \[([^\]]*)\]', keyd_body)[0],
      "a re-grant NEVER re-opens the whole keyspace")

# The binding is the load-bearing half — a rule bound to no subject is
# default-deny and silently inert, so assert the policy entry exists.
check('rules: ["lockdown-q-infra-keyd"]' in lk and 'subjects: ["infra-keyd"]' in lk,
      "each lockdown-q rule is BOUND to its subject (unbound = silent deny, F-177)")
check(lk.count('rules: ["lockdown-q-') == 2,
      "exactly one policy binding per declared subject (no over-generation)")
check(brace_balanced(lk), "brace/bracket balanced with lockdown re-grants")

# Flag ON with NO grants renders (the Ansible assert is what refuses), but must
# not silently invent a re-grant.
lk_bare = render(zenoh_identity_model="per-uuid-slot-pool", zenoh_slot_pool_salt=SALT,
                 zenoh_slot_pool_size=N, zenoh_edge_queryable_lockdown=True)
check("lockdown-q-" not in lk_bare,
      "flag ON with no declared grants invents nothing — the role's assert is the gate")


# ---------------------------------------------------------------------------
# Pre-cut PARTNER SLOT BLOCKS (ADR 1138). The ADR 1025 trick applied to the
# partner dimension: subjects for a whole range of slots exist BEFORE anyone
# holds the cert, so onboarding a partner into a pre-cut block needs no ACL
# edit and no router restart (F-156 — zenohd ignores SIGHUP for ACL reload,
# so "no restart" is the whole point).
# ---------------------------------------------------------------------------
print("== pre-cut partner slot blocks (ADR 1138) ==")

SALT = "test-salt"
POOL = 40
blocks = [
    {"lanes": ["REL-ALL"], "from": 30, "to": 32},
    {"lanes": ["REL-ALL", "REL-PARTNER"], "from": 33, "to": 34},
]
out = render(
    zenoh_identity_model="per-uuid-slot-pool",
    zenoh_slot_pool_salt=SALT,
    zenoh_slot_pool_size=POOL,
    zenoh_partner_read_partition=True,
    zenoh_partner_slot_blocks=blocks,
    zenoh_partner_read_lanes={},
)

block_all = [anchor(SALT, k) for k in (30, 31, 32)]
block_ukr = [anchor(SALT, k) for k in (33, 34)]
ordinary = [anchor(SALT, k) for k in (0, 1, 29, 35)]

for a in block_all + block_ukr:
    check(f'id: "partner-r-{a}"' in out, f"pre-cut block slot {a[:8]}… gets its partner read rule")
    check(f'id: "partner-{a}"' in out, f"pre-cut block slot {a[:8]}… gets its partner subject")

# The lane-set is what the block declares — and the UKR block must carry BOTH
# lanes, since a bilateral partner reads the broad lane too.
ukr_rule = out.split(f'id: "partner-r-{block_ukr[0]}"')[1].split("},")[0]
check("release/REL-ALL/**" in ukr_rule and "release/REL-PARTNER/**" in ukr_rule,
      "a two-lane block grants BOTH lanes to its slots")
all_rule = out.split(f'id: "partner-r-{block_all[0]}"')[1].split("},")[0]
check("release/REL-ALL/**" in all_rule and "release/REL-PARTNER/**" not in all_rule,
      "a one-lane block grants ONLY its lane — no leakage from the other block")

# The carve is only real if the anchor is ALSO removed from the fabric-wide
# read. This is the assertion that matters: a partner still enumerated in
# trial-peer-read reads everything and the whole partition is decorative.
tpr = out.split('id: "trial-peer-read"')[1].split("},")[0]
for a in block_all + block_ukr:
    check(f'"{a}"' not in tpr, f"pre-cut partner {a[:8]}… is EXCLUDED from the universal read")
for a in ordinary:
    check(f'"{a}"' in tpr, f"ordinary slot {a[:8]}… keeps the universal read")

# Explicit map and blocks compose, and the explicit entry WINS on collision —
# an operator's deliberate line about one anchor beats the block it falls in.
collide = anchor(SALT, 31)
out2 = render(
    zenoh_identity_model="per-uuid-slot-pool",
    zenoh_slot_pool_salt=SALT,
    zenoh_slot_pool_size=POOL,
    zenoh_partner_read_partition=True,
    zenoh_partner_slot_blocks=blocks,
    zenoh_partner_read_lanes={collide: ["ZAGROS"]},
)
crule = out2.split(f'id: "partner-r-{collide}"')[1].split("},")[0]
check("release/ZAGROS/**" in crule and "release/REL-ALL/**" not in crule,
      "an explicit per-anchor entry OVERRIDES the block it falls in")
check(out2.count(f'id: "partner-r-{collide}"') == 1,
      "a collision renders ONE rule, not two (duplicate ids would be a config error)")
check(out2.count(f'id: "partner-{collide}"') == 1,
      "a collision renders ONE subject, not two")

# Default-off: no blocks declared renders exactly today's output.
base = render(zenoh_identity_model="per-uuid-slot-pool", zenoh_slot_pool_salt=SALT,
              zenoh_slot_pool_size=POOL, zenoh_partner_read_partition=True,
              zenoh_partner_read_lanes={})
base_blocks = render(zenoh_identity_model="per-uuid-slot-pool", zenoh_slot_pool_salt=SALT,
                     zenoh_slot_pool_size=POOL, zenoh_partner_read_partition=True,
                     zenoh_partner_read_lanes={}, zenoh_partner_slot_blocks=[])
check(base == base_blocks, "an empty block list renders byte-identically to no block list")
# Match the RULE ID, not the bare string: the template's explanatory comments
# mention partner-r-<anchor> and render unconditionally, so a substring test
# passes for the wrong reason (this check failed on exactly that, first pass).
check('id: "partner-r-' not in base, "no partners declared ⇒ no partner RULE is invented")
check('id: "partner-' not in base, "no partners declared ⇒ no partner SUBJECT is invented")

# Per-vendor goatnets must be untouched by any of this.
pv = render(zenoh_identity_model="per-vendor", zenoh_partner_read_partition=True,
            zenoh_partner_slot_blocks=blocks)
check('id: "partner-r-' not in pv, "blocks are inert on a per-vendor goatnet (no slot pool to pre-cut)")

check(brace_balanced(out), "brace/bracket balanced with pre-cut partner blocks")

print("== federation ingress storage ==")
check("sovereign_from:" not in render(), "no peer means no federation store")
peers = [dict(sovereign="aaa", cn="fed.aaa.bbb.01", release_key_exprs=["from/bbb/**"]),
         dict(sovereign="ccc", cn="fed.ccc.bbb.01", release_key_exprs=["from/bbb/**"])]
for identity in ["per-vendor", "per-uuid-slot-pool"]:
    for count in [1, 2]:
        cfg = render(zenoh_identity_model=identity, zenoh_sovereign_peerings=peers[:count],
                     zenoh_from_retention_days=3, zenoh_slot_pool_size=2)
        check('dir: "/var/lib/zagros-zenoh/rocksdb/sovereign_from",' in cfg,
              f"{identity}/{count}: existing store path retained")
        for peer in peers[:count]:
            name = "sovereign_from_" + peer["sovereign"]
            match = re.search(r'"' + name + r'": \{(.*?)\n        \},', cfg, re.S)
            body = match.group(1) if match else ""
            check(f'key_expr: "from/{peer["sovereign"]}/**"' in body,
                  f"{identity}/{count}: peer subscription fits its edge grant")
            check(f'dir: "/var/lib/zagros-zenoh/rocksdb/{name}"' in body,
                  f"{identity}/{count}: distinct persistent path per peer")
            check('on_closure: "do_nothing"' in body and 'time_to_live: 259200000' in body,
                  f"{identity}/{count}: retains data and configured retention")
        check(cfg.count('"sovereign_from_') == count,
              f"{identity}/{count}: creates only the declared peer stores")
        check(brace_balanced(cfg), f"{identity}/{count}: balanced config")

print()
print("ALL RENDER CHECKS PASSED" if fails == 0 else f"{fails} CHECK(S) FAILED")
sys.exit(fails)
