#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
#
# "Was this identity EVER used?" — one command, all three layers.
#
# WHY THIS EXISTS. The question sounds like it should be one lookup and was
# three, none of which joined:
#
#   mesh credential  NetBird setup key `used_times > 0`   did they redeem it
#   mesh presence    NetBird peer attributed to them      are they on the mesh
#   login            Ory session history                  did they ever sign in
#
# Each layer is authoritative for itself and silent about the others, so the same
# population reads as a different number depending on which one you ask, and
# nobody can say which rows at `onboarded` have ever done anything at all.
#
# NEVER-USED is the AND of all three: no key redeemed, no peer, no session. That
# is the set safe to archive, because nothing about them exists to break.
#
# HONESTY RULES:
#   - A layer that cannot be reached reports UNKNOWN and the verdict becomes
#     UNKNOWN too — never "not used". An unreachable layer can make a dark
#     Enrolled column read as "nobody enrolled", and a scrub driven by a failed
#     probe deletes real people.
#   - peer -> person goes through the DEPLOYED resolver (ops/identity/peerowner,
#     POST /api/access/peers), never a name match written here. Peer names do
#     not reliably carry the grammar, so a local regex silently under-reports
#     presence, which is the direction that scrubs somebody who is on the mesh.
#
# usage:
#   identity-usage.py --env site-a                     # table, every proposal row
#   identity-usage.py --env site-a --json
#   identity-usage.py --env site-a --never-used-only   # the archive candidates
#
# --env names the goatnet (default $GOATNET; neither set is refused). The NetBird
# token is read from <GOATNET>_NB_API_TOKEN, else <GOATNET>_PORTAL_NB_SVC_TOKEN
# (site-a: SITE_A_...). The management API is $NB_API, else
# https://<public_mgmt_fqdn> from the goatnet's group_vars when the checkout is
# beside this file (ops/scripts/goatnet-facts.py mgmt-url).
#
# Run ON the mgmt host (needs kratos admin + the portal on the compose network).
import argparse
import json
import os
import ssl
import subprocess
import sys
import urllib.request

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import peerowner  # noqa: E402  the ONE peer->person client

NB_API = os.environ.get("NB_API", "").rstrip("/")
FACTS = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "scripts", "goatnet-facts.py")
KRATOS = os.environ.get("KRATOS_ADMIN_URL", "http://kratos:4434")
PORTAL = os.environ.get("PORTAL_INTERNAL_URL", "http://goat-zenoh-portal:8090")
CONSOLE_DB = os.environ.get("ONBOARD_DB", "/var/lib/goat-id-console/onboard.db")
# The SECOND register. An older `goat admin onboard` wrote a manifest here and no
# console row, so enumerating onboard.db alone misses everyone who came that way.
# A tool that answers "was this identity ever used" must enumerate the UNION or
# it silently answers for only part of the fleet.
MANIFEST_DIR = os.environ.get("OPERATOR_MANIFEST_DIR", "")

USED, NEVER, UNKNOWN = "used", "never-used", "unknown"


def _get(url, headers, timeout=30):
    req = urllib.request.Request(url, headers=headers)
    ctx = ssl.create_default_context()
    with urllib.request.urlopen(req, timeout=timeout, context=ctx) as r:
        return json.load(r)


def nb(path, token):
    return _get(NB_API + path, {"Authorization": "Token " + token})


def proposals(db):
    """id|handle|email|status for every proposal, read from the console store."""
    out = subprocess.run(
        ["docker", "run", "--rm", "-v", os.path.dirname(db) + ":/db:ro",
         "keinos/sqlite3:latest", "sqlite3", "-separator", "|",
         "/db/" + os.path.basename(db),
         "select id,handle,coalesce(email,''),status from onboard_proposals;"],
        capture_output=True, text=True)
    rows = []
    for line in out.stdout.splitlines():
        f = line.split("|")
        if len(f) >= 4:
            rows.append({"id": f[0], "handle": f[1], "email": f[2], "status": f[3]})
    return rows


def _manifest_email(path):
    """`identity.email` out of an operator manifest, without a YAML dependency.

    Deliberately a two-line scan rather than a parser: this runs on mgmt hosts
    where PyYAML is not guaranteed, and the one field it needs sits at a fixed
    depth under `identity:`. A miss returns "" and the login layer then reports
    UNKNOWN for that row — which is the correct outcome, since a row whose email
    we cannot read is a row whose sessions we cannot look up.
    """
    try:
        in_identity = False
        with open(path, encoding="utf-8", errors="replace") as source_file:
            for line in source_file:
                if line.startswith("identity:"):
                    in_identity = True
                    continue
                if in_identity:
                    if line and not line[0].isspace():
                        break                      # left the identity block
                    t = line.strip()
                    if t.startswith("email:"):
                        return t.split(":", 1)[1].strip().strip('"').strip("'")
    except OSError:
        pass  # unreadable file: no email
    return ""


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--json", action="store_true")
    ap.add_argument("--never-used-only", action="store_true")
    ap.add_argument("--archive-candidates", action="store_true",
                    help="rows at `onboarded` holding no LIVE unredeemed key — "
                         "the set that should not be sitting in that stage")
    ap.add_argument("--status", default="", help="only rows in this status")
    ap.add_argument("--env", default=os.environ.get("GOATNET", ""),
                    help="the goatnet (site-a) whose token and management API to use (default $GOATNET)")
    a = ap.parse_args()
    if not a.env:
        ap.error("--env is required (or set GOATNET)")

    global NB_API
    if not NB_API and os.path.isfile(FACTS):
        r = subprocess.run([sys.executable, FACTS, "mgmt-url", a.env],
                           capture_output=True, text=True)
        NB_API = r.stdout.strip() if r.returncode == 0 else ""
    if not NB_API.startswith("https://"):
        ap.error("no management API for %s: set NB_API=https://<public_mgmt_fqdn>" % a.env)

    prefix = a.env.upper().replace("-", "_")
    token = (os.environ.get(prefix + "_NB_API_TOKEN")
             or os.environ.get(prefix + "_PORTAL_NB_SVC_TOKEN", "")).strip()
    portal_key = os.environ.get("PORTAL_LOOKUP_TOKEN", "").strip()

    rows = proposals(CONSOLE_DB)
    known = {r["handle"] for r in rows}
    # Fold in the manifest-only population. `register` says which store
    # the row came from, so a reader can tell "this person has no console row"
    # from "this person's console row says onboarded" — two very different facts
    # that both render as an absence if you only look at one store.
    for r in rows:
        r["register"] = "console"
    if MANIFEST_DIR and os.path.isdir(MANIFEST_DIR):
        for fn in sorted(os.listdir(MANIFEST_DIR)):
            # Skip AppleDouble sidecars (`._name.yml`): a macOS tar drops one
            # beside every file, a shell glob hides them and os.listdir does not,
            # so they inflate the roster with phantom handles that resolve to
            # nothing. Cost of missing this: a count that looks like the fleet
            # grew by a third.
            if not fn.endswith(".yml") or fn.startswith("._"):
                continue
            h = fn[:-4]
            if h in known:
                continue
            rows.append({"id": "", "handle": h,
                         "email": _manifest_email(os.path.join(MANIFEST_DIR, fn)),
                         "status": "manifest-only", "register": "manifest"})
    if a.status:
        rows = [r for r in rows if r["status"] == a.status]

    # ---- layer 1: was a setup key ever redeemed -----------------------------
    keys_by_handle, keys_unknown = {}, False
    try:
        for k in nb("/api/setup-keys", token):
            n = k.get("name") or ""
            if n.startswith("peer-"):
                keys_by_handle.setdefault(n, k)
    except Exception as e:                                     # noqa: BLE001
        keys_unknown = True
        print("WARN setup keys unreadable (%s) — key column is UNKNOWN" % e, file=sys.stderr)

    def key_state(h):
        if keys_unknown:
            return UNKNOWN
        mine = [k for n, k in keys_by_handle.items() if n.startswith("peer-" + h + "-")]
        if not mine:
            return NEVER          # no credential was ever issued under this handle
        return USED if any(k.get("used_times", 0) > 0 for k in mine) else NEVER

    def has_live_key(h):
        """A key that is unredeemed AND still valid — i.e. this person can still
        act on it. THIS is what makes an `onboarded` row legitimate: freshly
        minted, not yet used, still usable. A row whose only keys are revoked or
        expired is not waiting for anybody; it is a record nobody can act on."""
        if keys_unknown:
            return None
        mine = [k for n, k in keys_by_handle.items() if n.startswith("peer-" + h + "-")]
        return any(k.get("used_times", 0) == 0 and k.get("valid")
                   and not k.get("revoked") for k in mine)

    # ---- layer 2: is there a peer, via the DEPLOYED resolver ----------------
    live_handles, peers_unknown = set(), False
    try:
        names = [p["name"] for p in nb("/api/peers", token)]
        res = peerowner.resolve(PORTAL, portal_key, names, timeout=30)
        if all(r["attribution"] == peerowner.UNREAD for r in res.values()) and res:
            peers_unknown = True
            print("WARN resolver unreachable — peer column is UNKNOWN", file=sys.stderr)
        live_handles = {r["handle"] for r in res.values() if r["handle"]}
    except Exception as e:                                     # noqa: BLE001
        peers_unknown = True
        print("WARN peers unreadable (%s) — peer column is UNKNOWN" % e, file=sys.stderr)

    def peer_state(h):
        if peers_unknown:
            return UNKNOWN
        return USED if h in live_handles else NEVER

    # ---- layer 3: did they ever sign in -------------------------------------
    sessions_by_email, sess_unknown = {}, False
    try:
        ids = _get(KRATOS + "/admin/identities?per_page=1000", {})
        for ident in ids:
            email = ((ident.get("traits") or {}).get("email") or "").lower()
            if not email:
                continue
            try:
                s = _get(KRATOS + "/admin/identities/%s/sessions" % ident["id"], {})
                sessions_by_email[email] = bool(s)
            except Exception:                                  # noqa: BLE001
                sessions_by_email[email] = None
    except Exception as e:                                     # noqa: BLE001
        sess_unknown = True
        print("WARN kratos unreadable (%s) — login column is UNKNOWN" % e, file=sys.stderr)

    def login_state(email):
        if sess_unknown or not email:
            return UNKNOWN
        v = sessions_by_email.get(email.lower())
        if v is None:
            return UNKNOWN if email.lower() in sessions_by_email else NEVER
        return USED if v else NEVER

    out = []
    for r in rows:
        k, p, l = key_state(r["handle"]), peer_state(r["handle"]), login_state(r["email"])
        if UNKNOWN in (k, p, l):
            verdict = UNKNOWN          # never call an unread layer "not used"
        elif USED in (k, p, l):
            verdict = USED
        else:
            verdict = NEVER
        out.append({**r, "key": k, "peer": p, "login": l, "verdict": verdict})

    for o in out:
        o["live_key"] = has_live_key(o["handle"])

    if a.never_used_only:
        out = [o for o in out if o["verdict"] == NEVER]
    if a.archive_candidates:
        # The stage should hold ONLY freshly minted accounts whose key is still
        # actionable. Anything else at `onboarded` is a record nobody can move:
        # the credential is dead, so the person cannot enrol even if they try,
        # and no amount of waiting changes the row. Those belong in cold storage,
        # restorable in one action. A row whose key state could not be READ is
        # excluded — the same rule as the verdict.
        # `manifest-only` rows are NEVER archive candidates: there is no console
        # row to archive, and their absence is F-357's gap rather than a
        # lifecycle state. Retiring them means creating the row first.
        # A LIVE PEER VETOES ARCHIVING, even with no live key. Someone can hold a
        # peer while their setup key reads unredeemed — they enrolled on a key
        # named outside the grammar, or on one since replaced — and shelving them
        # files away somebody who is on the mesh right now.
        # `peer` is UNKNOWN when the resolver could not be read,
        # and unknown is NOT permission to archive.
        out = [o for o in out
               if o["status"] == "onboarded"
               and o["live_key"] is False
               and o["peer"] == NEVER]

    if a.json:
        print(json.dumps(out, indent=2))
        return 0

    print("%-34s %-14s %-10s %-10s %-10s %s" %
          ("handle", "status", "key", "peer", "login", "VERDICT"))
    for o in out:
        print("%-34s %-14s %-10s %-10s %-10s %s" %
              (o["handle"][:34], o["status"], o["key"], o["peer"], o["login"],
               o["verdict"].upper()))
    n_never = sum(1 for o in out if o["verdict"] == NEVER)
    n_unk = sum(1 for o in out if o["verdict"] == UNKNOWN)
    print("\n%d row(s): %d never used, %d used, %d unknown"
          % (len(out), n_never, len(out) - n_never - n_unk, n_unk))
    if n_unk:
        print("UNKNOWN means a layer could not be read, NOT that it was unused. "
              "Do not archive on an unknown verdict.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
