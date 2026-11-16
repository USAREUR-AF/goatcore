#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
#
# Peer → responsible person, resolved against the portal's access graph.
#
# THE PYTHON CLIENT for `POST /api/access/peers`
# (docs/design/peer-identity-resolution.md, Option B). It is the third of three
# and deliberately the last one written by hand:
#
#   Go     the Thor resolver baking the relay-health card's "identity & access →"
#          link (ops/relay/health/relay-peer-names.py drives it; the card is Go)
#   Rust   panoscope_core::peerowner — netvis + panoscope
#   Python this module — the peerview surfaces and the Thor peer-name resolver
#
# WHY A SHARED MODULE AND NOT ANOTHER INLINE POST. The endpoint is shared; the
# READING of it was not. Its wire format distinguishes "handle matched but no
# live identity" from "the name carries no principal" only in prose — both are
# `resolved: false`, separated by whether `handle` is present — and it has no
# way at all to say "we could not ask", because a transport failure is the
# caller's to name. So every consumer that reads this API raw has to rediscover
# the same four states, and the ones that do not rediscover them render a blank
# that a reader takes for "unowned". Rust typed it; Go and Python each had their
# own partial version inline. This is that type, once, for Python.
#
# THE HONESTY RULE, carried verbatim from panoscope_core::peerowner and
# ultimately from the peer-name grammar SoT: *a peer name is EVIDENCE about
# ownership, never a record of it — the recorded linkage is `slots.identity_id`,
# stamped at onboard. A consumer that presents ownership must say which of the
# two it used.* Which is why this returns an attribution with every row and a
# caller can never receive a bare name to render as fact.
#
# NO PII LEAVES THIS MODULE BY DEFAULT. The endpoint is operator-scoped and
# returns the person's email and display name. `resolve()` keeps the Ory
# identity id (the deep-link key) and drops the rest unless asked, so a surface
# that renders handles cannot leak a roster of email addresses into a served
# json by forgetting to strip a field.

import json
import subprocess
import urllib.error
import urllib.request

# The four states. Never collapse them — each renders differently.
#
#   REGISTRY   the handle matched a roster member AND a live Ory identity shares
#              the roster's email-hash. The recorded linkage. Safe to present as
#              "who owns this"; carries the id-console deep-link.
#   NAME       the handle parsed and matched a roster member, but no live
#              identity did. Org known, person not. A lead, never an answer.
#   CUSTODY    the peer is a machine the portal enrolled (`machine-<handle>`);
#              `handle` is its CUSTODIAN, the person who enrolled it, and
#              `custody_of` is the machine. A machine cannot answer for itself.
#              `ory_id` is present only when the custodian is a live identity.
#   UNMATCHED  no declared identity matched. The resolver's `reason` says why,
#              and it is not always "no principal": a name can follow the
#              onboarding grammar and still miss a roster that has not caught
#              up with a runner mint. NOT "nobody is responsible".
#   UNREAD     we could not ask. Portal unreachable, unauthorized, or unset.
#              Distinct from UNMATCHED on purpose: "we did not look" and "we
#              looked and found no principal" are different facts.
REGISTRY = "registry"
NAME = "name"
CUSTODY = "custody"
UNMATCHED = "unmatched"
UNREAD = "unread"
ATTRIBUTIONS = (REGISTRY, NAME, CUSTODY, UNMATCHED, UNREAD)

EXPLAIN = {
    REGISTRY: "resolved from the registry (roster handle + live identity)",
    NAME: ("inferred from the peer name — org known, no live identity matched; "
           "treat as a lead, not an answer"),
    CUSTODY: "a machine the portal enrolled; this is the custodian who enrolled it",
    UNMATCHED: ("the name carries no principal — this peer did not come through "
                "the operator onboarding path"),
    UNREAD: "the resolver could not be asked, so nothing here is known either way",
}


def portal_base(url="", container="", network="mgmt", port="8090", log=None):
    """Where to reach the portal API from a mgmt host.

    An explicit URL wins. Otherwise docker-inspect the portal container's IP on
    its network, which is how the Thor resolver has reached it since 2026-07 and
    for a reason worth not rediscovering: the portal's mesh hostname does not
    resolve from the host itself, and the published :8088 is the oauth2/ADMINS
    gate rather than the app. The app answers an operator Bearer key on its own
    port inside the docker network, and the container IP changes on restart, so
    it is resolved per run rather than pinned.
    """
    if url:
        return url.rstrip("/")
    if not container:
        return ""
    try:
        ip = subprocess.check_output(
            ["docker", "inspect", "-f",
             '{{(index .NetworkSettings.Networks "' + network + '").IPAddress}}',
             container],
            text=True, timeout=10).strip()
    except Exception as e:                                   # noqa: BLE001
        if log:
            log(f"portal container ip resolve failed: {e}")
        return ""
    if not ip:
        if log:
            log(f"portal container {container} has no IP on net {network}")
        return ""
    return f"http://{ip}:{port}"


def resolve(base, api_key, names, timeout=20, keep_pii=False):
    """Resolve many peer names in ONE call. Never raises.

    Returns `{name: row}` for every name asked about — a transport failure comes
    back as an UNREAD row per name rather than as an exception, because
    ownership is an enrichment and a resolver being down must never take down
    the surface that called it (the rule `panoscope_core::peerowner::resolve`
    states: it never returns Err).

    Each row: `{attribution, handle, org, affiliation, nation, status, ory_id,
    id_console_path, reason, custody_of, owner}` — every key present, None where
    unknown. `custody_of` is the machine's handle when the peer is a machine.
    `owner` is the resolver's owner block (`owner_block`), or
    `{source: "unread", reason}` when the portal could not be asked.
    """
    names = [n for n in dict.fromkeys(n.strip() for n in names) if n]
    if not names:
        return {}
    if not base or not api_key:
        why = ("no portal URL resolved" if not base
               else "no portal API key configured")
        return {n: _row(UNREAD, reason=why) for n in names}

    try:
        req = urllib.request.Request(
            base.rstrip("/") + "/api/access/peers",
            data=json.dumps({"netbird_names": names}).encode(),
            method="POST",
            headers={"Authorization": "Bearer " + api_key,
                     "Content-Type": "application/json"})
        with urllib.request.urlopen(req, timeout=timeout) as r:
            body = json.load(r) or {}
    except urllib.error.HTTPError as e:
        return {n: _row(UNREAD, reason=f"portal answered HTTP {e.code}") for n in names}
    except Exception as e:                                   # noqa: BLE001
        return {n: _row(UNREAD, reason=f"portal unreachable ({e})") for n in names}

    peers = body.get("peers") or {}
    out = {}
    for n in names:
        out[n] = _read(peers.get(n), keep_pii)
    return out


def _row(attribution, **kw):
    row = {"attribution": attribution, "handle": None, "org": None,
           "affiliation": None, "nation": None, "status": None, "ory_id": None,
           "id_console_path": None, "reason": None, "custody_of": None,
           "owner": None}
    row.update(kw)
    if attribution == UNREAD and row["owner"] is None:
        # "We could not ask" is a state of its own on every surface, never an
        # absent owner: a refused key must not read as an unowned peer.
        row["owner"] = {"source": "unread", "reason": row["reason"]}
    return row


# The resolver's `owner` block, as a host surface may carry it. Everything here
# is an identifier or a state; the person's e-mail and display name are not in
# the block (they stay in the resolver's `person`, dropped by default above).
_OWNER_KEYS = ("source", "via", "matched_by", "link_note", "reason", "note", "machine_unit")
_OWNER_NESTED = {
    "person": ("handle", "identity_id", "disabled"),
    "org": ("id", "kind", "nation", "short_tag", "state", "reason"),
    "nationality": ("value", "state", "reason"),
}


def owner_block(res):
    """Where the owner came from, per docs/design/peer-identity-resolution.md
    §"The owner comes from the record, not the roster": `source` is `record`,
    `roster` or `none` (and `unread` when the call failed, set in `_row`).

    A portal older than that change sends no `owner`; its answers all came
    from the declared roster, and the block says so rather than letting a
    consumer read a roster answer as the record."""
    o = (res or {}).get("owner")
    if isinstance(o, dict):
        out = {k: o.get(k) for k in _OWNER_KEYS if o.get(k) not in (None, "")}
        for k, keys in _OWNER_NESTED.items():
            v = o.get(k)
            if isinstance(v, dict):
                out[k] = {kk: v.get(kk) for kk in keys if kk in v}
        return out
    if (res or {}).get("handle"):
        return {"source": "roster",
                "note": "the portal predates owner-from-record; this answer came from the declared roster"}
    return {"source": "none", "reason": (res or {}).get("reason")}


def _read(res, keep_pii):
    """One wire response → one graded row.

    The whole discrimination the wire format leaves in prose lives here: a
    missing `handle` is UNMATCHED, a `machine` block makes the handle a
    CUSTODIAN, a present handle without a live identity is NAME, and only a
    `resolved: true` carrying an Ory id earns REGISTRY.
    """
    if not res:
        return _row(UNREAD, reason="the resolver returned no row for this name")
    machine = (res.get("machine") or {}).get("system_handle") or None
    handle = res.get("handle") or None
    if not handle:
        return _row(UNMATCHED, reason=res.get("reason"), custody_of=machine,
                    owner=owner_block(res))
    person = res.get("person") or {}
    ory_id = person.get("ory_identity_id") or None
    if machine:
        attribution = CUSTODY
    else:
        attribution = REGISTRY if (res.get("resolved") and ory_id) else NAME
    row = _row(
        attribution,
        custody_of=machine,
        handle=handle,
        org=res.get("org"),
        affiliation=res.get("affiliation"),
        nation=res.get("nation"),
        status=res.get("status"),
        ory_id=ory_id,
        id_console_path=((res.get("deep_links") or {}).get("id_console_user")
                         or (f"/users/{ory_id}" if ory_id else None)),
        reason=res.get("reason"),
        owner=owner_block(res),
    )
    if keep_pii:
        row["email"] = person.get("email")
        row["display_name"] = person.get("name")
    return row
