#!/usr/bin/env python3
"""Revoke every peer still running Rosenpass and re-invite its owner.

DRY RUN BY DEFAULT. --apply is required to change anything.

WHY REVOKE. Setting `rosenpass_enabled: false` in group_vars stops NEW
enrollments and does nothing to a peer already enrolled: the flag lives in the
client's local config and survives `netbird down/up` AND re-enrollment with a
client's local config and survives `netbird down/up` AND re-enrollment with a
fresh setup key: a full down / up --setup-key cycle leaves "Quantum resistance:
true (permissive)" intact. Only an
explicit --enable-rosenpass=false clears it.

So a request changes nothing and a re-key alone changes nothing. What works is
revoking the peer (it stops working, so the owner must act) plus an enrollment
command whose text we control and which carries the disabling flags.

ORDER OF OPERATIONS, and why. Mint the replacement key BEFORE deleting the
peer. If minting fails we have disconnected nobody. Deleting first and then
failing to mint would strand someone with no way back.

WHAT THIS DOES NOT DO. It does not revoke the OLD setup key. Those keys are
mostly reusable and shared — revoking one to punish a single peer would break
unrelated working peers that were enrolled from it. Deleting the peer is the
revocation; the new key exists so the owner is handed the corrected command.

Run ON the mgmt host (needs docker for the console notify + the API token).

The goatnet is named by GOATNET (site-a) and its management API by NB_API
(https://<public_mgmt_fqdn>); ops/scripts/rosenpass-revoke-and-reinvite.sh sets
both from the goatnet's configuration. The NetBird token is read from the
onboard-runner's <GOATNET>_NB_API_TOKEN (site-a: SITE_A_NB_API_TOKEN).

usage (GOATNET and NB_API set):
  rosenpass_revoke_and_reinvite.py                 # dry run
  rosenpass_revoke_and_reinvite.py --apply
  rosenpass_revoke_and_reinvite.py --apply --only-offline   # safest first pass
  rosenpass_revoke_and_reinvite.py --apply --limit=3
"""
import json
import os
import re
import sqlite3
import subprocess
import sys
import time
import urllib.error
import urllib.request

APPLY = "--apply" in sys.argv
ONLY_OFFLINE = "--only-offline" in sys.argv
LIMIT = next((int(a.split("=")[1]) for a in sys.argv if a.startswith("--limit=")), None)

API = os.environ.get("NB_API", "").rstrip("/")
CONSOLE = os.environ.get("CONSOLE_URL", "http://127.0.0.1:8088")
# The goatnet's name as the console and the peer names carry it (site-a), and
# the prefix of its per-goatnet variables (SITE_A).
ENV_NAME = os.environ.get("GOATNET", "").replace("_", "-")
ENV_PREFIX = ENV_NAME.upper().replace("-", "_")
# NOT expanduser("~"): this runs under sudo, where ~ is /root and ~/rp-notice
# does not exist. that raised FileNotFoundError AFTER the mint,
# the delete and the notify had completed, so 20 owners were revoked and
# emailed while the run reported "0 revoked, 22 failed".
LEDGER = os.environ.get("RP_LEDGER", "/var/lib/peerview/rosenpass-revoked.log")
# Floor on the reissued key's device allowance. A person with one peer today
# still owns a phone, a rebuild, and the second attempt after installing in the
# wrong place; handing back exactly what was taken guarantees a support ticket
# the first time any of those happens. Matches the onboarding default.
REISSUE_MIN_DEVICES = int(os.environ.get("REISSUE_MIN_DEVICES", "5"))
NAMED = re.compile(r"^peer-(.+?)-" + re.escape(ENV_NAME) + r"-\d{4}-\d{2}-\d{2}[a-z]?-(.*)$")
KEY_NAMED = re.compile(r"^peer-(.+?)-" + re.escape(ENV_NAME) + r"-\d{4}-\d{2}-\d{2}")


def sh(*args):
    return subprocess.run(args, capture_output=True, text=True).stdout


def container_env(name, key):
    for line in sh("sudo", "docker", "inspect", name,
                   "--format", "{{range .Config.Env}}{{println .}}{{end}}").splitlines():
        if line.startswith(key + "="):
            return line.split("=", 1)[1]
    return ""


def api(path, method="GET", body=None, token=""):
    req = urllib.request.Request(API + path, method=method)
    req.add_header("Authorization", "Token " + token)
    req.add_header("Content-Type", "application/json")
    data = json.dumps(body).encode() if body is not None else None
    with urllib.request.urlopen(req, data, timeout=30) as r:
        raw = r.read()
        return json.loads(raw) if raw else {}


def notify(payload, token):
    req = urllib.request.Request(CONSOLE + "/api/notify", method="POST")
    req.add_header("Authorization", "Bearer " + token)
    req.add_header("Content-Type", "application/json")
    with urllib.request.urlopen(req, json.dumps(payload).encode(), timeout=30) as r:
        return r.status


def main():
    if not re.fullmatch(r"[a-z0-9][a-z0-9-]*", ENV_NAME):
        print("REFUSING: GOATNET names no goatnet (set GOATNET=<goatnet>, for example site-a).")
        return 2
    if not API.startswith("https://"):
        print("REFUSING: NB_API is not the goatnet's management URL (https://<public_mgmt_fqdn>).")
        return 2
    # onboard-runner's token FIRST: it is the identity that already mints setup
    # keys, so it demonstrably carries the authority this needs. The console's
    # ENROLL_WATCH_NB_TOKEN reads as the obvious choice and can be EMPTY,
    # which fails as a clean "no token available" rather than as a permission
    # error — check the length, not just the key's presence.
    nb_token = (container_env("onboard-runner", ENV_PREFIX + "_NB_API_TOKEN")
                or container_env("goat-id-console", "ENROLL_WATCH_NB_TOKEN")
                or os.environ.get(ENV_PREFIX + "_PORTAL_NB_SVC_TOKEN", "")).strip()
    notify_token = container_env("goat-id-console", "NOTIFY_TOKEN")

    # ── precondition ────────────────────────────────────────────────────────
    # The enrollment command in the reissue mail is rendered by the console from
    # NOTIFY_ROSENPASS. Grepping the binary for the disabling string proves the
    # code CAN emit it; it does not prove the deployed config WILL. If this were
    # true, the mail would tell 35 people to turn Rosenpass back ON — the exact
    # opposite of the point. Check the value, not the capability.
    rp = container_env("goat-id-console", "NOTIFY_ROSENPASS")
    if rp.strip().lower() != "false":
        print("REFUSING: console NOTIFY_ROSENPASS=%r, not 'false'." % rp)
        print("The reissue mail would instruct people to ENABLE Rosenpass.")
        return 2
    if not notify_token:
        print("REFUSING: console has no NOTIFY_TOKEN; the reissue mail cannot send.")
        return 2
    if not nb_token:
        print("REFUSING: no NetBird API token available.")
        return 2
    print("precondition OK — NOTIFY_ROSENPASS=false, notify + API tokens present")

    # ── live roster, re-read now ────────────────────────────────────────────
    # Anyone who complied since the notice went out must NOT be revoked. This is
    # the whole reason the roster is re-read at run time instead of reusing the
    # list the notice was addressed from.
    peers = api("/api/peers", token=nb_token)
    affected = [p for p in peers
                if str(p.get("extra_dns_labels") or "") or True
                and (p.get("rosenpass_enabled") or
                     (p.get("meta") or {}).get("rosenpass_enabled"))]
    # The API field name differs by version; fall back to the management DB view
    # that the census and the notice both used, so this cannot silently match 0.
    if not affected:
        affected = live_rosenpass_peers_from_db(peers)
    if ONLY_OFFLINE:
        affected = [p for p in affected if not p.get("connected")]
    if LIMIT:
        affected = affected[:LIMIT]

    if not affected:
        print("\nNo peer reports Rosenpass enabled. Nothing to do.")
        print("If that is a surprise, verify the query — an empty result here is")
        print("indistinguishable from a broken match, which is why it is stated.")
        return 0

    keys = {k["id"]: k for k in api("/api/setup-keys", token=nb_token)}
    enroller = enrolling_key_by_peer_name()
    handle_email = handle_email_map()

    # ── group by owner: one new key and one mail per person ─────────────────
    owners = {}
    for p in affected:
        m = NAMED.match(p["name"] or "")
        handle = m.group(1) if m else None
        if not handle:
            kn = keys.get(enroller.get(p["name"], ""), {}).get("name", "")
            km = KEY_NAMED.match(kn)
            handle = km.group(1) if km else None
        o = owners.setdefault(handle or "__UNATTRIBUTED__",
                              {"peers": [], "groups": set()})
        o["peers"].append(p)
        src = keys.get(enroller.get(p["name"], ""))
        if src:
            o["groups"].update(g["id"] if isinstance(g, dict) else g
                               for g in (src.get("auto_groups") or []))
        # FALLBACK, and it is not optional. Where the enrolling key cannot be
        # found (events.db has no row, or the key was deleted), auto_groups
        # comes back empty and the replacement key would be minted with NO
        # groups — re-enrolling the owner into nothing and silently stripping
        # every bit of access they had. Inherit the peer's CURRENT membership
        # instead, which is the thing being replaced.
        if not o["groups"]:
            # EXCLUDE the implicit "All" group — NetBird rejects it outright:
            #   {"message":"invalid auto groups: can't add 'all' group to the
            #    setup key","code":422}
            # A peer's CURRENT groups always contain All; a setup key's
            # auto_groups never do. Only this fallback path could produce a 422,
            # which is exactly why 20 owners went through and the
            # two that fell back here did not. Every peer is in All by
            # construction, so dropping it loses nothing.
            for g in (p.get("groups") or []):
                gid = g["id"] if isinstance(g, dict) else g
                gname = (g.get("name") if isinstance(g, dict) else "") or ""
                if gname.lower() == "all" or str(gid).endswith("_all"):
                    continue
                o["groups"].add(gid)

    print("\n%d peer(s) still on Rosenpass, across %d owner(s)"
          % (len(affected), len(owners)))
    recovered = recover_missing_emails(owners.keys(), handle_email)
    if recovered:
        print("  recovered %d owner(s) by name-matching a unique active identity: %s"
              % (len(recovered), ", ".join(sorted(recovered))))
        handle_email.update(recovered)
    unmailable = [h for h in owners if h == "__UNATTRIBUTED__" or h not in handle_email]
    if unmailable:
        print("  %d owner(s) have NO email and would be revoked with no way to tell"
              " them: %s" % (len(unmailable), ", ".join(sorted(unmailable))))

    if not APPLY:
        for h, o in sorted(owners.items()):
            mail = handle_email.get(h, "NO EMAIL")
            print("  %-42s %-32s %d peer(s), %d group(s)"
                  % (h, mail, len(o["peers"]), len(o["groups"])))
        print("\nDRY RUN — nothing changed. --apply to execute.")
        print("Consider --only-offline first: those peers are already"
              " disconnected, so revoking interrupts no session.")
        return 0

    done = set()
    if os.path.exists(LEDGER):
        with open(LEDGER) as source_file:
            done = {l.split("\t")[1] for l in source_file if "\t" in l}

    ok = fail = 0
    for h, o in sorted(owners.items()):
        if h in done:
            print("  skip (already done) %s" % h); continue
        email = handle_email.get(h)
        if not email:
            print("  SKIP %s — no email; revoking would strand them silently" % h)
            continue
        try:
            # 1. mint FIRST — a failure here disconnects nobody
            # REUSABLE, not one-off, and the count is not cosmetic. This step
            # DELETES every peer the owner has and hands them one key to get
            # back. A single-use key means they get exactly one device back:
            # anyone who runs two machines, retries after a fumbled first
            # attempt, or installs under WSL before realising they wanted the
            # native Windows client is stranded — and this runs against a
            # written deadline, so "come back to an operator" is not a recovery
            # path that scales. The first successful enrolment consumes a
            # one-off key permanently and every later attempt returns
            # "couldn't add peer: setup key is invalid".
            #
            # The allowance is deliberately at least the number of peers being
            # deleted, so nobody comes out of this with fewer devices than they
            # went in with.
            key_name = "peer-%s-%s-%s-rosenpass-reissue" % (h, ENV_NAME, time.strftime("%Y-%m-%d"))
            devices = max(REISSUE_MIN_DEVICES, len(o["peers"]))
            nk = api("/api/setup-keys", "POST", {
                "name": key_name,
                "type": "reusable", "expires_in": 604800, "usage_limit": devices,
                "auto_groups": sorted(o["groups"]), "ephemeral": False,
                # Without this the patched mgmt image has no prefix to combine
                # with the joining hostname, so the peer registers as bare
                # `<hostname>` — and the revoke selects peers by matching
                # `^peer-<handle>-<site>-`, so a bare-named peer is not cut and
                # the cut still reports success. Re-enrolling 38 owners with no
                # auto_peer_name would have manufactured 38 peers that a future
                # offboard cannot see. Same string as the key name: one naming
                # scheme, nothing to keep in sync.
                "auto_peer_name": key_name,
            }, token=nb_token)
            # 2. delete the peers
            for p in o["peers"]:
                api("/api/peers/" + p["id"], "DELETE", token=nb_token)
            # 3. hand over the corrected command
            notify({"event": "onboarded", "email": email, "name": h,
                    "env": ENV_NAME, "handle": h,
                    "setup_key": nk.get("key", "")}, notify_token)
            # The mint, the delete and the MAIL have all happened by here.
            # A ledger failure is NOT an action failure and must never be
            # reported as one: otherwise a FileNotFoundError makes the run print
            # "0 revoked, N failed" after revoking and emailing the owners. An
            # operator reading that re-runs, and because `done` is
            # read from the same missing ledger the re-run repeats everything —
            # a second key each and a second mail each.
            try:
                os.makedirs(os.path.dirname(LEDGER), exist_ok=True)
                with open(LEDGER, "a") as fh:
                    fh.write("%s\t%s\t%s\t%s\t%d peers\n" % (
                        time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
                        h, email, nk.get("id", "?"), len(o["peers"])))
            except Exception as log_exc:
                print("  WARNING %s WAS revoked, re-keyed and emailed, but the "
                      "ledger write failed (%s). Record it by hand before any "
                      "re-run." % (h, log_exc))
            ok += 1
            print("  revoked + reinvited  %-40s %d peer(s)" % (h, len(o["peers"])))
        except Exception as exc:
            fail += 1
            print("  FAILED %s — %s" % (h, exc))
        time.sleep(0.4)

    print("\n%d owner(s) revoked and reinvited, %d failed" % (ok, fail))
    return 1 if fail else 0


def live_rosenpass_peers_from_db(peers):
    """The management DB is the source the notice and the census both used."""
    out = sh("sudo", "docker", "exec", "postgres-a", "psql", "-U", "postgres",
             "-d", "netbird", "-At",
             "-c", "SELECT name FROM peers WHERE "
                   "(meta_flags::json->>'RosenpassEnabled')='true'")
    names = {l.strip() for l in out.splitlines() if l.strip()}
    return [p for p in peers if p.get("name") in names]


def enrolling_key_by_peer_name():
    """peer name -> the setup-key id that admitted it, out of the SQLite
    activity store. See F-343: this is NOT in Postgres despite a netbird_events
    database existing there."""
    sh("sudo", "docker", "cp", "management-a:/var/lib/netbird/events.db", "/tmp/rp-ev.db")
    sh("sudo", "chmod", "644", "/tmp/rp-ev.db")
    out = {}
    try:
        c = sqlite3.connect("/tmp/rp-ev.db")
        for _, init, meta in c.execute(
                "SELECT timestamp, initiator_id, meta FROM events WHERE activity=1"):
            try:
                m = json.loads(meta) if meta else {}
            except Exception:
                continue
            nm = m.get("name") or (m.get("fqdn") or "").split(".")[0]
            if nm and nm not in out:
                out[nm] = init
    except Exception as exc:
        print("  warning: events.db unreadable (%s) — group inheritance degraded" % exc)
    return out


def handle_email_map():
    out = sh("sudo", "docker", "run", "--rm", "-v",
             "/var/lib/goat-id-console:/d:ro", "alpine", "sh", "-c",
             "apk add -q sqlite && sqlite3 -separator '|' /d/onboard.db "
             "\"SELECT handle,lower(email) FROM onboard_proposals "
             "WHERE handle<>'' AND email<>''\"")
    m = {}
    for line in out.splitlines():
        if "|" in line:
            h, e = line.split("|", 1)
            m.setdefault(h.strip(), e.strip())
    return m


def kratos_identities():
    """email -> display name, for the active identities. Kratos has no handle
    trait, so this cannot be joined directly — it is the fallback for handles
    onboard.db never recorded."""
    out = sh("sudo", "docker", "exec", "postgres-a", "psql", "-U", "postgres",
             "-d", "kratos", "-At", "-F", "|",
             "-c", "SELECT lower(traits->>'email'), coalesce(traits->>'name','') "
                   "FROM identities WHERE state='active' "
                   "AND traits->>'email' IS NOT NULL")
    return dict(l.split("|", 1) for l in out.splitlines() if "|" in l)


def recover_missing_emails(handles, known):
    """Match a handle to exactly one active identity by name tokens.

    Owners of Rosenpass peers with no row in onboard.db, left
    unresolved, are silently SKIPPED — not revoked, so Rosenpass stays on
    and the remediation quietly misses them. Requiring a UNIQUE match matters:
    an ambiguous one would mail somebody else's setup key to the wrong person,
    which is worse than skipping.
    """
    idents = kratos_identities()
    out = {}
    for h in handles:
        if h in known or h == "__UNATTRIBUTED__":
            continue
        # DEDUPE. A handle like contributorg-demo-org-data-bennett repeats a
        # token, and scoring the repeat twice let an unrelated identity
        # (org29x) reach the 2-hit threshold on the surname alone
        # — turning a clean unique match into a false ambiguity, so the real
        # owner was skipped rather than mailed. Distinct tokens only.
        toks = sorted({t for t in re.split(r"[-_.]", h.lower())
                       if len(t) > 2 and t not in ("civ", "ctr", "usareur", "data")})
        hits = []
        for e, n in idents.items():
            hay = (e + " " + n).lower().replace(".", " ").replace("-", " ").replace("_", " ")
            if sum(1 for t in toks if t in hay) >= 2:
                hits.append(e)
        if len(hits) == 1:
            out[h] = hits[0]
        elif len(hits) > 1:
            print("  ambiguous: %s matches %d identities — NOT guessing" % (h, len(hits)))
    return out


if __name__ == "__main__":
    sys.exit(main())
