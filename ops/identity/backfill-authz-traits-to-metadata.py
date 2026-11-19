#!/usr/bin/env python3
"""Backfill Kratos authorization fields traits.{groups,org,aff,vendor} -> metadata_public.

Self-service Ory settings let an identity write any field in its traits schema,
so a user could self-grant a login group (traits.groups), which the patched
consent UI forwards into the JWT groups claim that fences every web surface —
a privilege escalation (docs/findings/F-572-*.md, confirmed on dev 2026-09-23).
The fix moves the authorization fields off self-editable traits into
admin-managed metadata_public, which the settings flow cannot write.

This script performs the data-at-rest half: for each identity it copies
traits.{groups,org,aff,vendor} into metadata_public.{...} (merging, never
clobbering a value already there) and removes the trait copy.

Deploy order (GHSA-9v3v — the readers are now fail-closed):
  Since GHSA-9v3v every reader (consent groups/aff claims, console, portal,
  ory-group-mirror, netbird reconcilers, goat CLI, identity-reconcile,
  backfill-aff-trait.py) reads ONLY metadata_public. A not-yet-backfilled
  identity therefore holds NO login groups, org or aff once those readers are
  deployed, so:
  1. Run this script FIRST, before deploying the fail-closed readers, or
     accept that unmigrated identities lose their groups until it runs.
     Every authorization value still in traits is one the holder could have
     written themselves through the settings flow while ory_authz_in_metadata
     was false, so NONE is promoted on the holder's word:
       - Without --trusted, --apply REFUSES (exit 3, nothing written) while
         any identity would promote a non-empty groups, org, aff or vendor.
         The dry-run lists each one as NEEDS-TRUSTED. There is no flag that
         promotes self-claimed values.
       - With --trusted FILE, a value is promoted only where the record backs
         it: a group the record lists for that identity, or an org/aff/vendor
         equal to the record's. Everything else is dropped from traits and
         printed as a DROP line (keep the output: it is the only record of the
         dropped values).
     The record is keyed by Kratos identity id, never by the self-editable
     traits.email:
       {"<identity-id>": {"email": "a@example.org", "groups": ["OBSERVER"],
                          "aff": {...}, "org": {...}, "vendor": {...}}}
     "email" is optional; when present it must equal the identity's
     traits.email, or nothing is promoted for that identity. Build it from
     the operator manifests / the console's audit of admin grants.
  2. Run with --trusted FILE --apply. Kratos validates the RESULT of a PATCH,
     so removing the traits leaves email/name (+ optional legacy vendor) which
     validate against either the old or the new schema; running this FIRST
     avoids a window where an identity still carrying traits.groups cannot
     save its settings under the new schema.
  3. Deploy the fail-closed readers, including the GHSA-9v3v consent image
     (kratos_selfservice_ui_image repinned to it; the ory role's
     consent-image-gate refuses a pre-fix image), then backfill-aff-trait.py (it now reads
     org only from metadata_public, so it needs step 2 done first).
  4. Re-run with --verify; when it prints READY, set ory_authz_in_metadata:
     true (deploys the new identity schema with groups/org/aff removed from
     traits; authz-backfill-gate.yml refuses the flip before READY).

Runs ON the mgmt host (Kratos admin API on 127.0.0.1:4434 by default). Stdlib
only — no pip installs on the Thor. Dry-run by default; pass --apply to write.

Reversibility: --reverse moves metadata_public.{...} back to traits.{...} (only
meaningful while the traits schema still accepts those fields, i.e. before the
step 3 schema deploy). Use it to roll the data back before rolling back the
schema.

Examples:
  python3 backfill-authz-traits-to-metadata.py --verify
  python3 backfill-authz-traits-to-metadata.py --trusted rec.json           # dry-run
  python3 backfill-authz-traits-to-metadata.py --trusted rec.json --apply   # migrate
  python3 backfill-authz-traits-to-metadata.py --reverse --apply   # roll back
"""
import argparse
import json
import re
import sys
import urllib.error
import urllib.parse
import urllib.request

DEFAULT_ADMIN = "http://127.0.0.1:4434"
PAGE_SIZE = 500
# The authorization fields that must live in admin-managed metadata_public,
# never in self-editable traits. "vendor" is the legacy org
# block (ADR 1052) and rides along so it is not left self-editable either.
AUTHZ_KEYS = ("groups", "org", "aff", "vendor")


def _req(method, url, body=None):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(url, data=data, method=method)
    req.add_header("Accept", "application/json")
    if data is not None:
        req.add_header("Content-Type", "application/json")
    with urllib.request.urlopen(req, timeout=30) as resp:
        raw = resp.read()
        return (json.loads(raw) if raw else None), dict(resp.headers)


def _next_link(headers):
    link = headers.get("Link") or headers.get("link") or ""
    for part in link.split(","):
        m = re.search(r"<([^>]+)>\s*;\s*rel=\"?next\"?", part)
        if m:
            return m.group(1)
    return None


def list_identities(admin):
    """Page through the Kratos admin identities list via the Link header."""
    out = []
    url = f"{admin}/admin/identities?page_size={PAGE_SIZE}"
    seen = set()
    while url:
        batch, headers = _req("GET", url)
        if batch:
            out.extend(batch)
        nxt = _next_link(headers)
        if not nxt:
            break
        url = urllib.parse.urljoin(admin, nxt)
        tok = urllib.parse.parse_qs(urllib.parse.urlparse(url).query).get("page_token", [""])[0]
        if tok in seen or not batch or len(batch) < PAGE_SIZE:
            break
        seen.add(tok)
    return out


def _nonempty(v):
    """True if v is a value worth moving (non-empty array/object/string)."""
    if isinstance(v, (list, dict, str)):
        return len(v) > 0
    return v is not None


def _trusted_entry(trusted, ident):
    """(entry, why_not) for an identity: the record's entry, matched by Kratos
    identity id. A secondary email, when the record carries one, must equal
    the identity's traits.email; a mismatch backs nothing."""
    entry = trusted.get(ident["id"])
    if entry is None:
        return None, "identity id not in trusted record"
    want = entry.get("email")
    if want is not None and str(want).strip().lower() != str(ident["email"]).strip().lower():
        return None, f"trusted record email {want!r} disagrees with traits.email"
    return entry, None


def plan(identities, src_container, dst_container, forward, trusted=None):
    """Return actions moving AUTHZ_KEYS from src_container to dst_container.

    An action carries the merged dst object, the list of keys to remove from
    src, what was promoted, what was dropped (with why) and, without a trusted
    record, which keys would need one.

    - forward (traits -> metadata_public): a key already in metadata_public is
      kept. Once metadata_public carries a groups key the identity is
      migrated, and any authorization value in traits was written by the
      holder after that; it is removed, never copied.
      For a not-yet-migrated identity every traits value is self-claimed
      (GHSA-9v3v). Without `trusted` a non-empty one is NOT promoted and is
      listed in `needs_trust` (main() refuses --apply while any exist). With
      `trusted` ({identity_id: {"email"?, "groups", "aff", "org", "vendor"}})
      a group is promoted only if the identity's entry lists it, and an
      org/aff/vendor only if it equals the entry's value; the rest is dropped.
    - reverse (metadata_public -> traits): the admin-managed value always wins
      over whatever traits hold.

    Forward, an identity with no groups anywhere still gets an explicit
    `groups: []` written to dst, the migration marker every reader and
    --verify look for.
    """
    actions, skipped = [], []
    for i in identities:
        src = i.get(src_container) or {}
        dst = dict(i.get(dst_container) or {})
        ident = {"id": i["id"], "email": (i.get("traits") or {}).get("email", "?")}
        migrated = forward and "groups" in dst
        move_keys = [k for k in AUTHZ_KEYS if k in src]
        moved, promoted, dropped, needs_trust = [], {}, [], []
        entry, why_not = (None, None)
        if forward and trusted is not None:
            entry, why_not = _trusted_entry(trusted, ident)
        for k in move_keys:
            if not forward:
                dst[k] = src[k]
                moved.append(k)
            elif migrated:
                moved.append(f"{k}(DROPPED: written to traits after migration)")
                if _nonempty(src[k]):
                    dropped.append((k, src[k], "written to traits after migration"))
            elif k in dst:
                moved.append(f"{k}(kept existing)")
                if _nonempty(src[k]) and src[k] != dst[k]:
                    dropped.append((k, src[k], "metadata_public already holds an admin-managed value"))
            elif not _nonempty(src[k]):
                moved.append(f"{k}(empty, removed)")
            elif trusted is None:
                needs_trust.append(k)
                moved.append(f"{k}(NEEDS-TRUSTED)")
            elif entry is None:
                dropped.append((k, src[k], why_not))
                moved.append(f"{k}(DROPPED)")
            elif k == "groups":
                have = src[k] if isinstance(src[k], list) else []
                allowed = set(entry.get("groups") or [])
                # Traits are holder-controlled: a non-string item (an object,
                # a number) is never a group name; drop and report it rather
                # than hash it.
                bad = [g for g in have if not isinstance(g, str)]
                names = [g for g in have if isinstance(g, str)]
                keep = [g for g in names if g in allowed]
                drop = [g for g in names if g not in allowed]
                dst[k] = keep
                if keep:
                    promoted[k] = keep
                if drop or not isinstance(src[k], list):
                    dropped.append((k, drop if isinstance(src[k], list) else src[k], "not in trusted record"))
                if bad:
                    dropped.append((k, bad, "not a group name (non-string item)"))
                moved.append("groups" + ("(PARTLY DROPPED)" if drop or bad else ""))
            elif k in entry and entry[k] == src[k]:
                dst[k] = src[k]
                promoted[k] = src[k]
                moved.append(k)
            else:
                dropped.append((k, src[k], "not in trusted record"))
                moved.append(f"{k}(DROPPED)")
        seeded = False
        if forward and "groups" not in dst:
            dst["groups"] = []
            seeded = True
        if not move_keys and not seeded:
            skipped.append({**ident, "why": f"nothing to move and groups already present in {dst_container}"})
            continue
        if seeded:
            moved.append("groups=[] (seed)")
        actions.append({**ident, "dst": dst, "remove": move_keys, "moved": moved,
                        "promoted": promoted, "dropped": dropped, "needs_trust": needs_trust})
    return actions, skipped


def load_trusted(path):
    """Read the trusted record: {identity_id: {"email"?, "groups"?, "aff"?,
    "org"?, "vendor"?}}. The pre-GHSA-9v3v email-keyed shape
    ({email: [groups]}) is refused: email is self-editable."""
    with open(path) as fh:
        raw = json.load(fh)
    if not isinstance(raw, dict):
        raise SystemExit("--trusted: the record must be a JSON object keyed by Kratos identity id")
    out = {}
    for ident, entry in raw.items():
        if not isinstance(entry, dict):
            raise SystemExit(f"--trusted: entry {ident!r} must be an object keyed by identity id "
                             '({"email"?, "groups", "aff", "org", "vendor"}), not an email-keyed list')
        bad = set(entry) - {"email", *AUTHZ_KEYS}
        if bad:
            raise SystemExit(f"--trusted: entry {ident!r} has unknown keys {sorted(bad)}")
        groups = entry.get("groups", [])
        if not isinstance(groups, list) or not all(isinstance(g, str) for g in groups):
            raise SystemExit(f"--trusted: entry {ident!r} groups must be a list of strings")
        out[str(ident)] = entry
    return out


def apply_action(admin, act, src_container, dst_container):
    """JSON Patch: set the merged dst container, remove each moved src field."""
    patch = [{"op": "add", "path": f"/{dst_container}", "value": act["dst"]}]
    for k in act["remove"]:
        patch.append({"op": "remove", "path": f"/{src_container}/{k}"})
    _req("PATCH", f"{admin}/admin/identities/{act['id']}", patch)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--admin-url", default=DEFAULT_ADMIN, help=f"Kratos admin API (default {DEFAULT_ADMIN})")
    ap.add_argument("--apply", action="store_true", help="perform writes (default: dry-run)")
    ap.add_argument("--reverse", action="store_true", help="move metadata_public -> traits (rollback)")
    ap.add_argument("--verify", action="store_true", help="report identity counts by field state and exit")
    ap.add_argument("--trusted", metavar="FILE",
                    help='JSON {"<identity-id>": {"email"?, "groups", "aff", "org", "vendor"}}: promote only '
                         'traits values this record backs; required for --apply to promote anything (GHSA-9v3v)')
    args = ap.parse_args()
    trusted = load_trusted(args.trusted) if args.trusted else None

    src_container, dst_container = (
        ("metadata_public", "traits") if args.reverse else ("traits", "metadata_public")
    )

    try:
        identities = list_identities(args.admin_url)
    except urllib.error.URLError as e:
        print(f"ERROR: cannot reach Kratos admin at {args.admin_url}: {e}", file=sys.stderr)
        return 2

    if args.verify:
        def has_authz(container):
            c = container or {}
            return any(k in c and _nonempty(c[k]) for k in AUTHZ_KEYS)
        in_traits = sum(1 for i in identities if has_authz(i.get("traits")))
        in_meta = sum(1 for i in identities if has_authz(i.get("metadata_public")))
        no_meta_groups = sum(1 for i in identities
                             if "groups" not in (i.get("metadata_public") or {}))
        print(f"identities:                        {len(identities)}")
        print(f"authz fields in traits:            {in_traits}")
        print(f"authz fields in metadata_public:   {in_meta}")
        print(f"missing metadata_public.groups:    {no_meta_groups}")
        ready = in_traits == 0 and no_meta_groups == 0
        print("READY — no authz fields in traits and every identity has metadata_public.groups"
              if ready else
              f"NOT ready — traits authz on {in_traits}, missing metadata groups on {no_meta_groups}")
        return 0 if ready else 1

    actions, skipped = plan(identities, src_container, dst_container, forward=not args.reverse, trusted=trusted)
    mode = "APPLY" if args.apply else "DRY-RUN"
    print(f"[{mode}] {src_container} -> {dst_container}: {len(actions)} to migrate, "
          f"{len(skipped)} skipped (of {len(identities)} identities)\n")

    untrusted = [a for a in actions if a["needs_trust"]]
    if untrusted:
        print(f"{len(untrusted)} identities carry self-claimed authorization values in traits that only a "
              "trusted record can promote (GHSA-9v3v):")
        for a in untrusted:
            src = next(i for i in identities if i["id"] == a["id"]).get("traits") or {}
            for k in a["needs_trust"]:
                print(f"  NEEDS-TRUSTED {a['id']} {a['email']} {k}={json.dumps(src[k], sort_keys=True)}")
        if args.apply:
            print("\nREFUSED: nothing written. Re-run with --trusted FILE (keyed by identity id); "
                  "values the record does not back are dropped and reported.", file=sys.stderr)
            return 3
        print()

    ok = err = 0
    for a in actions:
        line = f"  {a['email']:40s} move={','.join(a['moved'])}"
        for k, v in a["promoted"].items():
            # A traits value becoming admin-managed, backed by the trusted record.
            line += f"\n  {'':40s} PROMOTE {k}={json.dumps(v, sort_keys=True)} (trusted record)"
        for k, v, why in a["dropped"]:
            line += f"\n  {'':40s} DROP {k}={json.dumps(v, sort_keys=True)} ({why})"
        if not args.apply:
            print(line)
            continue
        try:
            apply_action(args.admin_url, a, src_container, dst_container)
            ok += 1
            print(f"OK{line}")
        except urllib.error.HTTPError as e:
            err += 1
            print(f"FAIL {a['email']}: HTTP {e.code} {e.read().decode()[:200]}", file=sys.stderr)

    if args.apply:
        print(f"\napplied: {ok} ok, {err} failed")
        return 1 if err else 0
    print("\n(dry-run — re-run with --apply to write)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
