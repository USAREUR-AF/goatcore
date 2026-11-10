# SPDX-License-Identifier: Apache-2.0
#
# intent.py — building and reading IntentV1.
#
# One document per governed goatnet at ops/consistency/intent/<goatnet>/
# intent.json: the RESOLVED answer to "what is this goatnet supposed to run",
# with the resolution state attached to every value. Canon:
# docs/design/consistency-foundation.md §3 (ADR 1110 D1).
#
# It is rendered as the SECOND output of the same resolution pass that renders
# group_vars/<member>/pins.yml — one resolver, two renderings, the pattern
# ops/dns/generate-mesh-hosts.py established when it emitted both the zone file
# and the name snapshot.

import datetime
import hashlib
import json
import os

from .paths import INTENT_SCHEMA, LITE_TABLE, TABLE, intent_path, rel
from .resolve import (SchemaError, all_scope_vars, artifact_type, caps_files,
                      deviation_scope, member_scopes, resolve, resolve_var_full,
                      role_defaults)

INTENT_FORMAT = "goat-intent-v1"

# How long a fetched copy of a generation may be treated as fresh without
# re-fetching. 30 days is the island window (ADR 0206) — a site whose WAN has
# been down that long is the case the artifact has to survive, not fail. It
# bounds COPY freshness only: a node holding the current generation is `match`
# however old that generation is (design doc §5.3).
TTL_SECONDS = 2592000

# The three fields generation identity is computed WITHOUT (§3.3).
UNHASHED = ("serial", "generated_at", "content_hash")

# How many predecessor generations the artifact carries identity triples for
# (§3.3a). Ten is a declared constant, not a tuning knob: the tail is evidence
# for grading, and a node further behind than this is honestly not accountable
# to anything a reader holding one artifact can see.
HISTORY_DEPTH = 10


def canonical_json(obj):
    """THE serialization contract. Keys sorted, two-space indent, UTF-8, one
    trailing newline — CF-5 signs SHA-256 over these exact bytes (ADR 1082's
    pattern reused whole), so any change here invalidates every signature and is
    a format change, not a formatting change."""
    return json.dumps(obj, sort_keys=True, indent=2, ensure_ascii=False) + "\n"


def sha256_of(data):
    return "sha256:" + hashlib.sha256(data).hexdigest()


def sha256_of_file(path):
    with open(path, "rb") as f:
        return sha256_of(f.read())


def content_hash(body):
    """SHA-256 over the canonical body, excluding the generation-identity
    fields. A deterministic function of the tree that both sides can compute —
    which is why it, and not the git commit, is the comparison key: two commits
    routinely produce identical intent, and comparing on commit would grade
    every node diverged after every unrelated merge."""
    return sha256_of(canonical_json(
        {k: v for k, v in body.items() if k not in UNHASHED}).encode("utf-8"))


def one_line(text):
    """YAML block scalars carry newlines; JSON rows carry sentences."""
    return " ".join(str(text or "").split())


def declared_block(scope, dev):
    out = {"scope": scope, "reason": one_line(dev.get("reason"))}
    if dev.get("until"):
        out["until"] = str(dev["until"])
    return out


def var_cell(r):
    """A Resolution as an artifact cell. The state travels with the value
    because a consumer holding only the artifact cannot recompute it — it does
    not have the tree — and the honesty grammar dies at the first hop if the
    state is dropped (§3.1)."""
    if r.state == "resolved":
        return {"state": "resolved", "value": r.key, "source": r.source}
    if r.state == "unreadable":
        cell = {"state": "unreadable", "detail": r.detail}
        if r.source:
            cell["source"] = r.source
        return cell
    return {"state": "unset", "detail": r.detail}


def var_row(row, members, goatnet, scopes, all_scope, defaults, cap_devs):
    var = row["var"]
    out = {
        "var": var,
        "why": one_line(row.get("why")),
        "members": {m: var_cell(resolve_var_full(var, scopes[m], all_scope, defaults))
                    for m in members},
    }
    declared = {scope: declared_block(scope, dev or {})
                for scope, dev in (cap_devs.get(var) or {}).items()
                if scope in members or scope == goatnet}
    if declared:
        out["declared"] = declared
    return out


def resolve_ca_id(spec, members, scopes, all_scope, defaults):
    """The goatnet's offline-CA id, resolved from the group_vars key the table
    NAMES (`ca_id_var:`) rather than one this code guesses — the two governed
    goatnets hold it under different key names.

    A member that does not set it does not contradict one that does (zagros-b
    inherits zagros's CA). Two members resolving it to DIFFERENT values is a real
    finding, and renders null rather than a coin-flip.

    F-395 — "does not set it" has to mean the MEMBER did not set it, not merely
    that resolution produced nothing. `resolve_var_full` falls through to role
    defaults, and the role default for the governed key `portal_cert_sign_ca_id`
    is the NOVA ca id. So a member that runs no portal — every joiner, by
    ADR 1084 D3 — still resolved a value, disagreed with its founder, and
    collapsed the goatnet's ca_id to null. The symptom lands far away: the
    generator writes the intent happily and `intent-sign` then refuses the whole
    goatnet with "intent has no ca_id".

    Role-default hits are therefore skipped. A default is the absence of a
    declaration, which is precisely the case this function already meant to
    ignore."""
    var = (spec or {}).get("ca_id_var")
    if not var:
        return None
    values = set()
    for m in members:
        r = resolve_var_full(var, scopes[m], all_scope, defaults)
        if r.state != "resolved" or not r.key:
            continue
        if str(r.source or "").startswith("role default:"):
            continue
        values.add(r.key)
    return values.pop() if len(values) == 1 else None


def sources_of(goatnet, lite_goatnets=frozenset(), table=None, lite_table=None):
    """The input files this goatnet's generation was rendered from, each as the
    sha256 of its exact bytes. A goatnet declared in fleet-pins.yaml: the fleet
    table and the capabilities file — unchanged since CF-1, so a lite render
    beside it moves no committed intent. A goatnet declared in the lite sidecar
    (ADR 1213): the fleet table, the sidecar and the capabilities file. Each
    capabilities extension a deployment supplies (resolve.caps_files) is listed
    after the capabilities file."""
    table = table or TABLE
    out = {rel(table): sha256_of_file(table)}
    if goatnet in lite_goatnets:
        lite_table = lite_table or LITE_TABLE
        out[rel(lite_table)] = sha256_of_file(lite_table)
    for caps_file in caps_files():
        out[rel(caps_file)] = sha256_of_file(caps_file)
    return out


def build_body(goatnet, goatnets, components, caps, member_of, allow, outside,
               lite_goatnets=frozenset(), table=None, lite_table=None):
    """Everything except serial / generated_at / content_hash — i.e. exactly the
    part that is hashed. `lite_goatnets` is the sidecar's declared set
    (resolve.PinTable.lite_goatnets); it decides the `sources` block only."""
    spec = goatnets[goatnet] or {}
    members = list(spec.get("members") or [])
    all_scope = all_scope_vars()
    defaults = role_defaults()
    scopes = member_scopes(members)
    cap_devs = caps.get("deviations") or {}

    comps = []
    for cname, cspec in components.items():
        entry = {
            "name": cname,
            "var": cspec["var"],
            "artifact_type": artifact_type(cspec),
            "members": {},
        }
        # The declared join key to a node's ADR 1060 deploy ledger (CF-3). It is
        # carried only when the table SAYS something — an absent key means "join
        # on the component name", and an explicit null means "this component has
        # no ledger coverage and can never be observed live". Those are three
        # different statements and the artifact keeps them apart, because
        # defaulting the third into the first would turn a known coverage gap
        # into a node that merely looks like it is not reporting.
        if "ledger_component" in cspec:
            entry["ledger_component"] = cspec["ledger_component"]
        # CF-4: the declared render watch-set — WHICH rendered files are this
        # component's config identity. Paths stay LITERAL (the {{ ghcr_org }}
        # rule): only the deploying host can resolve them, so a reader joins
        # render records at component granularity and uses the declared COUNT
        # as the coverage denominator. The expected hash deliberately does not
        # appear here — a per-host render is a function of host vars and env
        # lookups no repo-side reader may evaluate; the node's own converge
        # record (the render manifest) is the baseline the comparison uses.
        if cspec.get("render_watch"):
            entry["render_watch"] = list(cspec["render_watch"])
        if cspec.get("note"):
            entry["note"] = one_line(cspec["note"])
        for m in members:
            value, dev = resolve(cspec, m, goatnet)
            scope = deviation_scope(cspec, m, goatnet)
            if value is None:
                entry["members"][m] = {"state": "absent",
                                       "declared": declared_block(scope, dev or {})}
                continue
            # `{{ ghcr_org }}` stays LITERAL and `resolved`: the same committed
            # string must resolve to a different org depending on which checkout
            # deploys, so it is carried un-evaluated rather than guessed.
            cell = {"state": "resolved", "value": value}
            if dev is not None:
                cell["declared"] = declared_block(scope, dev)
            entry["members"][m] = cell
        comps.append(entry)

    retired = []
    for row in caps.get("retired") or []:
        var = row["var"]
        present = [m for m in members if var in scopes[m]]
        if var in all_scope:
            present.append("group_vars/all.yml")
        retired.append({"var": var, "why": one_line(row.get("why")), "present_in": present})

    ungoverned = [
        {"member": m, "var": k, "source": path,
         "allowlisted": ("%s:%s" % (m, k)) in allow}
        for m, k, path in sorted(outside) if member_of.get(m) == goatnet
    ]

    return {
        "format": INTENT_FORMAT,
        "goatnet": goatnet,
        "members": members,
        "ttl_seconds": TTL_SECONDS,
        "ca_id": resolve_ca_id(spec, members, scopes, all_scope, defaults),
        "sources": sources_of(goatnet, lite_goatnets, table, lite_table),
        "components": comps,
        "capabilities": [var_row(r, members, goatnet, scopes, all_scope, defaults, cap_devs)
                         for r in (caps.get("capabilities") or [])],
        "node_shape": [var_row(r, members, goatnet, scopes, all_scope, defaults, cap_devs)
                       for r in (caps.get("node_shape") or [])],
        "retired": retired,
        "ungoverned": ungoverned,
    }


def read_committed(path):
    """The committed artifact, or {} when there isn't a usable one. A malformed
    file is treated as absent rather than fatal: the regenerate-diff below is
    what reports it, with a diff a reviewer can read."""
    if not os.path.exists(path):
        return {}
    try:
        with open(path) as f:
            data = json.load(f)
    except ValueError:
        return {}
    return data if isinstance(data, dict) else {}


def load(goatnet):
    """Read a committed generation, VERIFIED AT READ (CF-5). THE way a consumer
    gets intent — nobody other than this package's builder re-resolves the
    tree, and nobody gets a body whose signature and content_hash did not check
    out. Fail-closed (ADR 1109 P14): any unverifiable state — missing artifact,
    missing or invalid signature, tampered body, a context that cannot verify —
    raises IntentVerifyError with the refusal message, and the caller's honest
    output is that message, never a grade."""
    from .signing import IntentVerifyError, verify_file

    path = intent_path(goatnet)
    try:
        with open(path, "rb") as f:
            raw = f.read()
    except OSError as e:
        raise IntentVerifyError(
            "no readable intent artifact for goatnet '%s' at %s (%s)"
            % (goatnet, path, e))
    try:
        doc = json.loads(raw)
    except ValueError as e:
        raise IntentVerifyError("%s is not JSON: %s" % (path, e))
    if not isinstance(doc, dict):
        raise IntentVerifyError("%s is not an IntentV1 document" % path)
    v = verify_file(path, doc=doc, raw=raw)
    if v.state != "verified":
        raise IntentVerifyError(v.detail)
    stated, actual = self_consistent(doc)
    if stated != actual:
        raise IntentVerifyError(
            "%s carries content_hash %s but its body hashes to %s — refusing "
            "to grade against an intent nobody can trust" % (path, stated, actual))
    return doc


def identity(doc):
    """A generation's identity triple — everything a reader needs to RECOGNISE a
    generation and nothing of what it contained. Carrying bodies would make an
    old generation targetable, which §3.3a refuses: the tail is evidence for
    grading, never a target for converging."""
    return {"serial": doc["serial"],
            "content_hash": doc["content_hash"],
            "generated_at": doc["generated_at"]}


def has_identity(doc):
    """Whether a committed document has a usable identity to be succeeded by. A
    malformed or absent predecessor contributes no history entry rather than a
    guessed one — the tail says only what it can see."""
    return (isinstance(doc.get("serial"), int) and doc["serial"] >= 1
            and isinstance(doc.get("content_hash"), str) and doc["content_hash"]
            and isinstance(doc.get("generated_at"), str) and bool(doc["generated_at"]))


def succeed(committed):
    """The history tail a new generation inherits: the PRIOR generation's triple
    prepended to the prior tail, truncated at HISTORY_DEPTH."""
    tail = [h for h in (committed.get("history") or []) if isinstance(h, dict)]
    if has_identity(committed):
        tail = [identity(committed)] + tail
    return tail[:HISTORY_DEPTH]


def render(body, committed, now=None):
    """Attach the history tail and the generation identity.

    `serial` and `generated_at` move ONLY when the body changes, which is what
    makes regeneration with no change a byte-no-op and therefore what makes
    --check a freshness gate rather than a clock.

    Since §3.3a, `history` is part of the body — so a serial bump now also
    changes the body, through the tail it inherits. That is correct rather than
    circular: the no-op test below asks whether the body INCLUDING the committed
    tail still hashes to the committed hash, and only a generation that is
    genuinely succeeding another one grows a tail entry."""
    keep = dict(body)
    keep["history"] = [h for h in (committed.get("history") or []) if isinstance(h, dict)]
    digest = content_hash(keep)
    serial = committed.get("serial")
    generated_at = committed.get("generated_at")
    unchanged = (committed.get("content_hash") == digest
                 and isinstance(serial, int) and serial >= 1
                 and isinstance(generated_at, str) and generated_at)
    if unchanged:
        doc = keep
    else:
        doc = dict(body)
        doc["history"] = succeed(committed)
        digest = content_hash(doc)
        serial = (serial + 1) if isinstance(serial, int) and serial >= 1 else 1
        generated_at = (now or datetime.datetime.now(datetime.timezone.utc)).strftime(
            "%Y-%m-%dT%H:%M:%SZ")
    doc = dict(doc)
    doc["serial"] = serial
    doc["generated_at"] = generated_at
    doc["content_hash"] = digest
    return canonical_json(doc), doc


def self_consistent(doc):
    """Whether a committed document's content_hash is the hash of its own body.
    One layer of the §3.4 integrity story: the CF-5 signature proves WHO
    produced the bytes, this proves the body still IS those bytes' claim about
    itself, and the regenerate-diff proves the tree still agrees. A reader
    checks the first two; CI checks all three."""
    return doc.get("content_hash"), content_hash(doc)


def validate_schema(doc, where):
    """Schema-validate against the committed JSON Schema. Hard-required rather
    than best-effort: a validation step that silently skips when a dependency is
    missing is a gate that reports green for the wrong reason."""
    try:
        import jsonschema
    except ImportError:
        raise SchemaError("jsonschema required to render/validate the intent "
                          "artifact (pip install jsonschema)")
    with open(INTENT_SCHEMA) as f:
        schema = json.load(f)
    try:
        jsonschema.validate(instance=doc, schema=schema)
    except jsonschema.ValidationError as e:
        loc = "/".join(str(p) for p in e.absolute_path) or "(document root)"
        raise SchemaError("%s fails %s at %s: %s"
                          % (where, rel(INTENT_SCHEMA), loc, e.message))
