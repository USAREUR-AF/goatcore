#!/usr/bin/env python3
"""Validate federation partner declarations (ADR 1157).

These files carry the trust level for each federated sovereign. The review of a
change to one IS the access-control decision, so the checks here are about
making a bad declaration impossible to merge quietly rather than about schema
tidiness.

Run: python3 ops/federation/validate-partners.py [--partners DIR]
"""
import datetime
import os
import re
import sys

try:
    import yaml
except ImportError:
    print("validate-partners: PyYAML not available; skipping", file=sys.stderr)
    sys.exit(0)

HERE = os.path.dirname(os.path.abspath(__file__))
PARTNERS = os.path.join(HERE, "partners")
if "--partners" in sys.argv:
    PARTNERS = sys.argv[sys.argv.index("--partners") + 1]
LEVELS = ("recognised", "connected", "exchanging")
REQUIRED = ("sovereign_id", "level", "partner_cn", "ingress", "egress", "rationale")

# Names our own fabric services use. A partner presenting one of these is the
# F-424 condition: the ACL matches a Common Name and cannot see the issuer, so
# a partner CN that collides with ours inherits our subject's policy.
OURS = {
    "goat-zenoh-router", "operator-trial-peer", "fabric-health-trial-peer",
    "grant-keyd-trial-peer", "audit-subscriber", "audit-subscriber-trial-peer",
    "ntp-health-pub-trial-peer", "goat-time-health-trial-peer",
    "goat-mesh-health-trial-peer", "goat-presence-health-trial-peer",
    "goat-node-net-health-trial-peer", "goat-consistency-health-trial-peer",
    "goat-fabric-crawler-trial-peer", "goat-consistency-console-trial-peer",
}

# A peering forwarder, when one side listens through one (ADR 1218,
# docs/design/peering-forwarder.md §6). Optional; when present, every key is required.
FORWARDER_KEYS = ("operator", "placement", "endpoint")
PLACEMENTS = ("cloud-vm", "owned-site")
ENDPOINT = re.compile(r"([A-Za-z0-9.-]+|\[[0-9A-Fa-f:]+\]):([0-9]{1,5})")

# The acceptance result, published only through review of a change to this file
# (docs/design/federation-acceptance.md §7). It records evidence; it never widens
# what `level` grants, and an expired or drifted record goes stale, it does not
# rewrite anything.
ACCEPTANCE_STATUS = ("exchanging", "stale", "degraded")
ACCEPTANCE_KEYS = ("status", "session", "manifest_sha256", "agreement_sha256", "local_bundle_sha256",
                   "peer_bundle_sha256", "combined_sha256", "verified_at", "expires", "approved_by")
DIGEST = re.compile(r"sha256:[0-9a-f]{64}")

errs, notes, seen_cn, seen_id, n = [], [], {}, {}, 0


def bad(f, msg):
    errs.append(f"  {os.path.basename(f)}: {msg}")


for fn in sorted(os.listdir(PARTNERS)) if os.path.isdir(PARTNERS) else []:
    if not fn.endswith((".yaml", ".yml")):
        continue
    path = os.path.join(PARTNERS, fn)
    example = fn.startswith("EXAMPLE-")
    with open(path, encoding="utf-8") as fh:
        try:
            d = yaml.safe_load(fh) or {}
        except yaml.YAMLError as e:
            bad(path, f"not valid YAML: {e}")
            continue
    n += 1

    for k in REQUIRED:
        if k not in d or d[k] in (None, "", []):
            bad(path, f"missing required field '{k}'")
    if errs and any(os.path.basename(path) in e for e in errs):
        continue

    if d["level"] not in LEVELS:
        bad(path, f"level must be one of {LEVELS}, got {d['level']!r}")

    sid = str(d["sovereign_id"])
    if not re.fullmatch(r"[a-z0-9][a-z0-9-]{1,15}", sid):
        bad(path, f"sovereign_id {sid!r} must be lowercase alphanumeric/hyphen, 2-16 chars")

    cn = str(d["partner_cn"])
    if cn in OURS:
        bad(path, f"partner_cn {cn!r} collides with one of OUR service names. "
                  "The ACL matches a CN and cannot see the issuer: a partner "
                  "presenting this would inherit that subject's policy.")
    if cn in seen_cn and not example:
        bad(path, f"partner_cn {cn!r} already declared in {seen_cn[cn]}")
    if not example:
        seen_cn[cn] = fn
        if sid in seen_id:
            bad(path, f"sovereign_id {sid!r} already declared in {seen_id[sid]}")
        seen_id[sid] = fn

    for field in ("ingress", "egress"):
        for k in d.get(field) or []:
            if k.strip() in ("**", "**/**", "*"):
                bad(path, f"{field} {k!r} is a root wildcard — grant exact prefixes only")
            if field == "ingress" and not str(k).startswith(f"from/{sid}/"):
                bad(path, f"ingress {k!r} must sit under from/{sid}/ so data is "
                          "attributable by prefix, which is the only authenticated "
                          "attribution available across a boundary")

    if "forwarder" in d:
        fw = d["forwarder"]
        if not isinstance(fw, dict):
            bad(path, "forwarder must be a mapping with operator, placement and endpoint")
        else:
            for k in FORWARDER_KEYS:
                if fw.get(k) in (None, ""):
                    bad(path, f"forwarder.{k} is required when a forwarder is declared")
            for k in sorted(set(fw) - set(FORWARDER_KEYS)):
                bad(path, f"forwarder.{k} is not a forwarder field ({', '.join(FORWARDER_KEYS)})")
            op = fw.get("operator")
            if op and not re.fullmatch(r"[a-z0-9][a-z0-9-]{1,15}", str(op)):
                bad(path, f"forwarder.operator {op!r} must be the sovereign_id of the party that runs it")
            pl = fw.get("placement")
            if pl and pl not in PLACEMENTS:
                bad(path, f"forwarder.placement must be one of {PLACEMENTS}, got {pl!r}")
            ep = fw.get("endpoint")
            if ep:
                m = ENDPOINT.fullmatch(str(ep))
                if not m or not 1 <= int(m.group(2)) <= 65535:
                    bad(path, f"forwarder.endpoint {ep!r} must be host:port, the address the "
                              "partner dials, as transport_offer.endpoint carries it")

    if "acceptance" in d:
        ac = d["acceptance"]
        if not isinstance(ac, dict):
            bad(path, "acceptance must be a mapping, as acceptance_session.py propose writes it")
        else:
            for k in sorted(set(ac) - set(ACCEPTANCE_KEYS)):
                bad(path, f"acceptance.{k} is not an acceptance field ({', '.join(ACCEPTANCE_KEYS)})")
            st = ac.get("status")
            if st not in ACCEPTANCE_STATUS:
                bad(path, f"acceptance.status must be one of {ACCEPTANCE_STATUS}, got {st!r}")
            if st == "exchanging":
                for k in ACCEPTANCE_KEYS:
                    if ac.get(k) in (None, "", {}):
                        bad(path, f"acceptance.{k} is required when acceptance.status is exchanging")
                if d.get("level") != "exchanging":
                    bad(path, f"acceptance.status is exchanging but level is {d.get('level')!r}: "
                              "the evidence cannot claim more than the grant")
                ap = ac.get("approved_by")
                if ap not in (None, "", {}) and (not isinstance(ap, dict) or len(ap) != 2 or sid not in ap
                                                 or not all(str(v).strip() for v in ap.values())):
                    bad(path, f"acceptance.approved_by must name the approver of each side, {sid} and ours")
            for k in ("manifest_sha256", "agreement_sha256", "local_bundle_sha256", "peer_bundle_sha256",
                      "combined_sha256"):
                if ac.get(k) and not DIGEST.fullmatch(str(ac[k])):
                    bad(path, f"acceptance.{k} must be sha256:<64 hex digits>")
            exp = str(ac.get("expires") or "")
            try:
                if exp and datetime.date.fromisoformat(exp[:10]) < datetime.date.today():
                    notes.append(f"  {fn}: acceptance evidence expired on {exp[:10]}; it is stale. Run a new "
                                 "bilateral session; the grant (level) is not changed by this note.")
            except ValueError:
                bad(path, f"acceptance.expires {exp!r} is not a date (YYYY-MM-DD)")

    if len(str(d["rationale"]).strip()) < 20:
        bad(path, "rationale is too short to be a review artifact")

print(f"[federation-partners] {n} declaration(s) checked")
if notes:
    print("[federation-partners] note")
    print("\n".join(notes))
if errs:
    print("[federation-partners] FAIL")
    print("\n".join(errs))
    print("\n  See docs/adr/1157-cross-sovereign-federation-edge-terminated.md")
    sys.exit(1)
print("[federation-partners] PASS")
