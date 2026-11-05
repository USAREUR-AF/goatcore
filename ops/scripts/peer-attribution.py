#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""peer-attribution.py — whose device is this, when the name does not say.

WHY THIS EXISTS
    Offboarding finds a person's devices by matching peer names against
    `^peer-<handle><tag>-<site>-`.  A peer whose name does not match is not
    found, not deleted, and stays on the mesh with its groups — and the play
    reports success.  That is a silent miss on the one path where a miss means
    a credential that was supposed to be cut is still live.

    Peer names are assembled by whoever mints the key.  Two of the three mint
    paths set `auto_peer_name`; the portal's machine-enrolment mint does not,
    and a hand-mint through the NetBird dashboard cannot.  So off-grammar peers
    keep appearing, and each one is invisible to the cut.

    The management server already knows the answer.  Every setup-key enrolment
    writes a `peer.setupkey.add` event carrying the peer id (`target_id`) and
    the key that admitted it (`meta.setup_key_name`).  That link does not care
    what the peer is called.  Measured on zagros 2026-08-17: the name grammar
    covers 95 of 116 peers, the event log covers 115, and the union covers all
    116 — the two signals are complementary, so use both.

WHAT IT DOES NOT DO
    It does not cut anything and it does not guess.  For a handle it reports
    candidates in tiers with the evidence for each, and exits non-zero when a
    candidate exists that the revoke regex would not match — because that, not
    the absence of a device, is the condition worth failing on.

USAGE
    ops/scripts/peer-attribution.py <goatnet> --audit
        Every peer, how it is attributable, and which are attributable to
        nobody.

    ops/scripts/peer-attribution.py <goatnet> --handle <handle> [--site <site>]
        Candidates for one person.  Exit 1 if any candidate would be missed by
        a name-matching cut.  This is the preflight for an offboard.

    Token comes from <SITE>_NB_API_TOKEN, matching operator-onboard.yml.
    Reading events requires an admin token.

CAVEAT — EVENT RETENTION
    The event log is finite.  A peer enrolled before the oldest retained event
    has no `peer.setupkey.add` to find, and falls back to name evidence only.
    The tool prints the retention window so a clean audit is not mistaken for
    proof when the window is short.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import urllib.error
import urllib.request

# Mirrors netbird_site_url_map in ops/ansible/operator-onboard.yml.
SITE_URLS = {
    "zagros": "https://netbird.zagros.example",
    "zagros-prod": "https://netbird.zagros-prod.example",
    "vega": "https://mgmt.vega.netbird-prod.goat.example",
}


def api(base: str, token: str, path: str):
    req = urllib.request.Request(
        base + path,
        headers={"Authorization": f"Token {token}", "Accept": "application/json"},
    )
    try:
        with urllib.request.urlopen(req, timeout=45) as r:
            return json.load(r)
    except urllib.error.HTTPError as e:
        body = e.read().decode("utf-8", "replace")[:200]
        sys.exit(f"{path} -> HTTP {e.code}: {body}\n"
                 f"(listing setup keys and events needs an ADMIN token)")


def key_by_peer(events: list) -> dict:
    """peer id -> the setup key name that admitted it.

    `peer.setupkey.add` is written at enrolment and is the only record tying a
    peer to a key; the peer object itself carries neither a key reference nor,
    for key-enrolled peers, a user id.
    """
    out = {}
    for ev in events:
        if ev.get("activity_code") == "peer.setupkey.add":
            name = (ev.get("meta") or {}).get("setup_key_name") or ""
            # Keep the FIRST enrolment for a peer id; later events on the same
            # id are group edits and logins, not admissions.
            out.setdefault(ev.get("target_id"), name.strip())
    return out


def stem(key_name: str) -> str:
    """The handle-ish part of a setup key name.

    Minted keys are `peer-<handle>[-<tag>]-<site>-<date>`; hand-mints are
    whatever the operator typed. Strip the one prefix we control and leave the
    rest for prefix comparison rather than pretending to parse it.
    """
    n = (key_name or "").strip()
    return n[5:] if n.startswith("peer-") else n


def tier(peer: dict, key_name: str, handle: str, site: str) -> tuple[str, str] | None:
    """Which tier of evidence ties this peer to this handle, if any."""
    name = peer.get("name") or ""
    # Tier 1 — what the revoke regex itself matches. Both grammar forms, per
    # the dual accept in operator-onboard.yml R1.2 (pre-v1.1.0 peers carry a
    # doubled handle segment).
    for prefix in (f"peer-{handle}-{site}-", f"peer-{handle}-{handle}-{site}-"):
        if name.startswith(prefix):
            return ("exact", f"peer name starts with {prefix!r}")
    # Tiers 2 and 3 — the peer name says nothing; the key that admitted it might.
    s = stem(key_name)
    if not s:
        return None
    if s.startswith(f"{handle}-{site}-") or s == handle:
        return ("key-exact", f"admitted by key {key_name!r}")
    if s.startswith(handle) or handle.startswith(s):
        return ("key-fuzzy", f"admitted by key {key_name!r} (stem {s!r} shares a prefix with the handle)")
    return None


def cmd_audit(peers, keymap, events):
    if events:
        ts = [e.get("timestamp", "") for e in events if e.get("timestamp")]
        print(f"event window: {min(ts)[:19]} .. {max(ts)[:19]}  ({len(events)} events)")
    grammar = [p for p in peers if (p.get("name") or "").startswith("peer-")]
    keyed = [p for p in peers if keymap.get(p["id"])]
    union = [p for p in peers if (p.get("name") or "").startswith("peer-") or keymap.get(p["id"])]
    print(f"peers: {len(peers)}   name-grammar: {len(grammar)}   key-attributable: {len(keyed)}   "
          f"either: {len(union)}   NEITHER: {len(peers) - len(union)}")
    print()
    off = sorted((p for p in peers if not (p.get("name") or "").startswith("peer-")),
                 key=lambda z: (z.get("name") or "").lower())
    if off:
        print(f"OFF-GRAMMAR PEERS ({len(off)}) — invisible to a name-matching cut:")
        print(f"  {'PEER NAME':32} {'OS':15} {'CONN':5} ADMITTED BY")
        for p in off:
            k = keymap.get(p["id"]) or "** no enrolment event — unattributable **"
            conn = "up" if p.get("connected") else "down"
            print(f"  {(p.get('name') or '')[:31]:32} {(p.get('os') or '')[:14]:15} {conn:5} {k}")
        print()
    orphan = [p for p in peers
              if not (p.get("name") or "").startswith("peer-") and not keymap.get(p["id"])]
    if orphan:
        print(f"UNATTRIBUTABLE ({len(orphan)}) — no name evidence and no enrolment event.")
        print("These cannot be tied to a person by any signal the mesh holds.")
        return 1
    print("Every peer is attributable by at least one signal.")
    return 0


def cmd_handle(peers, keymap, handle, site):
    hits = []
    for p in peers:
        t = tier(p, keymap.get(p["id"], ""), handle, site)
        if t:
            hits.append((t[0], t[1], p))
    if not hits:
        print(f"no peers attributable to {handle!r} on site {site!r}.")
        return 0

    order = {"exact": 0, "key-exact": 1, "key-fuzzy": 2}
    hits.sort(key=lambda h: (order[h[0]], (h[2].get("name") or "").lower()))
    missed = [h for h in hits if h[0] != "exact"]

    print(f"peers attributable to {handle!r} on {site!r}: {len(hits)}")
    print()
    for t, why, p in hits:
        mark = "CUT " if t == "exact" else "MISS"
        conn = "up" if p.get("connected") else "down"
        print(f"  [{mark}] {(p.get('name') or '')[:38]:39} {(p.get('os') or '')[:13]:14} {conn:5} {t}")
        print(f"         {why}")
    print()
    if missed:
        print(f"REFUSING: {len(missed)} peer(s) belong to this person but do NOT match the")
        print("name pattern the revoke cuts on, so an offboard would leave them connected")
        print("and still report success. Rename them to the grammar, or delete them by id,")
        print("before running the revoke:")
        for _, _, p in missed:
            print(f"   DELETE /api/peers/{p['id']}   # {p.get('name')}")
        return 1
    print("All attributable peers match the cut pattern; a revoke will reach every one.")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("goatnet")
    ap.add_argument("--audit", action="store_true",
                    help="report attribution for every peer on the goatnet")
    ap.add_argument("--handle", help="report candidates for one person (offboard preflight)")
    ap.add_argument("--site", help="site token used in peer names (default: the goatnet)")
    ap.add_argument("--url", help="NetBird management URL (default: SITE_URLS[goatnet]); "
                    "operator-onboard.yml passes its own netbird_site_url_map entry")
    a = ap.parse_args()

    if not a.audit and not a.handle:
        ap.error("choose --audit or --handle")

    base = a.url or SITE_URLS.get(a.goatnet)
    if not base:
        ap.error(f"unknown goatnet {a.goatnet!r}; add it to SITE_URLS or pass --url")

    var = a.goatnet.upper().replace("-", "_") + "_NB_API_TOKEN"
    token = os.environ.get(var, "")
    if not token:
        sys.exit(f"no admin token in ${var}")

    peers = api(base, token, "/api/peers")
    events = api(base, token, "/api/events")
    keymap = key_by_peer(events)

    if a.audit:
        return cmd_audit(peers, keymap, events)
    return cmd_handle(peers, keymap, a.handle, a.site or a.goatnet)


if __name__ == "__main__":
    sys.exit(main())
