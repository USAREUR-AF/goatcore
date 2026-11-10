#!/usr/bin/env python3
"""Retroactive onboarding backfill (run ON the mgmt host, next to the console).

Everyone onboarded BEFORE the three-stage system existed has a Kratos login
but no onboarding record, so the console's provenance view has nothing to
show for them. This script creates one `onboarded` proposal row per such
identity, marked `retroactive = 1` — the UI then explains that unrecorded
fields (sponsor, approvals, tier) predate the buildout instead of rendering
blanks that read like bugs.

A handle is never guessed (#1244). Kratos traits carry no handle, so the
email's local-part is only a CANDIDATE for the fabric handle. The 2026-07
zagros run wrote candidates as handles: 15 of its 22 rows named handles the
fabric never knew, and the enrollment watcher's prefix match (pre-F-358, and
its known-handle blind spot for exactly this population) promoted them to
`enrolled`, where nothing demotes them. Every writing mode of this script
now verifies a handle against a roster export before it lands: a candidate
absent from the roster is refused and listed, and --handle-map supplies the
operator-confirmed handle instead.

Modes (one per run):
  (default)      Backfill: one `onboarded` row per Kratos identity that has
                 no proposal row, every handle verified against --roster.
  --check        Verify every `enrolled` row's handle against --roster.
                 Exits 1 when a RETROACTIVE row fails: those rows were
                 written from live identities, so absence means the row
                 never named a real handle. Non-retroactive rows are
                 reported, never failed: an identity may leave the fabric
                 after enrolling, and that is lifecycle, not corruption.
  --fix-handles  Repair `prop-retro-*` rows whose handle is not in the
                 roster, from --handle-map. The UPDATE is guarded on id,
                 current handle and retroactive=1; dry-run unless --apply.
  --self-test    Run the pure logic on inline fixtures. No network, no db.

Inputs:
  --roster FILE      One fabric handle per line, `#` comments allowed.
                     Export it from the roster the queue must join against
                     (`goat who`, or the portal registry's org listing).
  --handle-map FILE  Lines of `email,handle`, `#` comments allowed. The
                     handle must itself be in the roster.

Idempotent: an identity whose email already has ANY proposal row is skipped,
so re-runs and the live flow never collide. Dry-run by default; --apply
writes. Exit 0 clean, 1 when anything was refused or a check failed, 2 for
usage or environment faults.

Usage (on the host):
  sudo python3 retro-backfill.py --db /var/lib/goat-id-console/onboard.db \
      --kratos http://127.0.0.1:4434 --env zagros --roster roster.txt [--apply]

Requires the console build that added the retroactive/sponsor_approved_*
columns (the console migrates the schema at startup — start it once first).
"""
import argparse
import hashlib
import json
import re
import sqlite3
import sys
import urllib.request

NOTE = "retroactive import — onboarded before the three-stage system existed; provenance fields were never recorded"


def fetch_identities(base):
    # Kratos v1.x token-paginates; a `page=` param silently returns []. One
    # large fetch + a hard stop if it might have truncated keeps this honest
    # without implementing Link-header paging.
    per_page = 500
    url = f"{base}/admin/identities?per_page={per_page}"
    with urllib.request.urlopen(url, timeout=15) as r:
        out = json.load(r)
    if len(out) >= per_page:
        sys.exit(f"got {len(out)} identities == per_page cap — possible truncation; "
                 "raise per_page or implement Link-header paging before trusting this run")
    return out


def handle_candidate(email):
    """The email's local-part shaped like a handle. A CANDIDATE only: it is
    written when the roster confirms it and refused when it does not."""
    local = email.split("@", 1)[0].lower()
    return re.sub(r"[^a-z0-9-]+", "-", local).strip("-") or "unknown"


def iso_to_epoch(s):
    import datetime
    try:
        return int(datetime.datetime.fromisoformat(s.replace("Z", "+00:00")).timestamp())
    except Exception:
        return 0


def load_roster(text):
    """One handle per line; blank lines and # comments ignored."""
    out = set()
    for line in text.splitlines():
        line = line.strip()
        if line and not line.startswith("#"):
            out.add(line)
    return out


def load_handle_map(text):
    """Lines of `email,handle`; blank lines and # comments ignored."""
    out = {}
    for n, line in enumerate(text.splitlines(), 1):
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        if "," not in line:
            sys.exit(f"--handle-map line {n}: want `email,handle`, got {line!r}")
        email, handle = (p.strip() for p in line.split(",", 1))
        if not email or not handle:
            sys.exit(f"--handle-map line {n}: empty email or handle")
        out[email.lower()] = handle
    return out


def resolve_handle(email, roster, handle_map):
    """The verified fabric handle for an email.

    Returns (handle, None) when the mapped or derived handle is in the
    roster, (None, refused_candidate) when it is not. A mapped handle
    outside the roster is refused too: the map corrects the guess, the
    roster is still the authority.
    """
    mapped = handle_map.get(email.lower())
    if mapped is not None:
        return (mapped, None) if mapped in roster else (None, mapped)
    cand = handle_candidate(email)
    return (cand, None) if cand in roster else (None, cand)


def plan_backfill(ids, existing, excludes, roster, handle_map, env):
    """Rows to write, refusals to list, count skipped as already-recorded."""
    todo, refused, skipped = [], [], 0
    for i in ids:
        email = (i.get("traits", {}).get("email") or "").strip()
        if not email:
            skipped += 1
            continue
        if email.lower() in existing or email.lower() in excludes:
            skipped += 1
            continue
        handle, cand = resolve_handle(email, roster, handle_map)
        if handle is None:
            refused.append((email, cand))
            continue
        todo.append((
            "prop-retro-" + hashlib.sha256(email.lower().encode()).hexdigest()[:12],
            iso_to_epoch(i.get("created_at", "")) or 0,
            "", "",                      # sponsor, sponsor_name — never recorded
            env,
            handle,
            email,
            "onboarded",
            "{}", "{}",                  # manifest_json, lint_json
            None, None,                  # decided_by, decided_at — never recorded
            NOTE,
            0, 1, "[]", "",              # escalated, complete, petitions, tier
            1,                           # retroactive
        ))
    return todo, refused, skipped


def classify_enrolled(rows, roster):
    """Split enrolled rows whose handle is not in the roster.

    rows: (id, handle, email, retroactive). Retroactive rows in the result
    are corruption (the row never named a real handle); the rest are
    reported for a human to read, because an identity that left after
    enrolling looks exactly the same from here.
    """
    retro, normal = [], []
    for id_, handle, email, retroactive in rows:
        if handle in roster:
            continue
        (retro if retroactive else normal).append((id_, handle, email))
    return retro, normal


def plan_fixes(rows, roster, handle_map, taken):
    """Guarded updates for retro rows whose handle the roster does not know.

    rows: (id, handle, email) already filtered to handle ∉ roster.
    taken: handles already on other proposal rows — a mapped handle that
    collides is refused rather than silently shared.
    """
    updates, unfixable = [], []
    claimed = set(taken)
    for id_, handle, email in rows:
        new = handle_map.get(email.lower())
        if new is None:
            unfixable.append((id_, handle, email, "no mapping for this email"))
        elif new not in roster:
            unfixable.append((id_, handle, email, f"mapped handle {new!r} is not in the roster"))
        elif new in claimed:
            unfixable.append((id_, handle, email, f"mapped handle {new!r} is already on another proposal row"))
        else:
            claimed.add(new)
            updates.append((id_, handle, new))
    return updates, unfixable


# ---------------------------------------------------------------- self-test

ROSTER_FIXTURE = {"jane-doe", "g1x-gcs", "dave-2"}
MAP_FIXTURE = {"jane.doe@example.invalid": "jane-doe", "sam@example.invalid": "nobody-here"}


def self_test():
    cases = [
        ("candidate: dots and case fold to a handle shape",
         handle_candidate("Jane.Doe@example.invalid") == "jane-doe"),
        ("candidate: specials collapse, edges strip",
         handle_candidate("a_b+c@x") == "a-b-c"),
        ("resolve: a mapped handle in the roster wins",
         resolve_handle("jane.doe@example.invalid", ROSTER_FIXTURE, MAP_FIXTURE) == ("jane-doe", None)),
        ("resolve: a mapped handle outside the roster is refused",
         resolve_handle("sam@example.invalid", ROSTER_FIXTURE, MAP_FIXTURE) == (None, "nobody-here")),
        ("resolve: an unmapped candidate in the roster passes",
         resolve_handle("g1x-gcs@example.invalid", ROSTER_FIXTURE, {}) == ("g1x-gcs", None)),
        ("resolve: an unmapped candidate outside the roster is refused",
         resolve_handle("jane@example.invalid", ROSTER_FIXTURE, {}) == (None, "jane")),
        ("check: a retroactive miss fails, a normal miss reports",
         classify_enrolled(
             [("prop-retro-1", "jane", "jane.doe@example.invalid", 1),
              ("prop-2", "gone-person", "gone@example.invalid", 0),
              ("prop-retro-3", "g1x-gcs", "g1x-gcs@example.invalid", 1)],
             ROSTER_FIXTURE) == ([("prop-retro-1", "jane", "jane.doe@example.invalid")],
                                 [("prop-2", "gone-person", "gone@example.invalid")])),
        ("fix: mapped + rostered + free plans an update",
         plan_fixes([("prop-retro-1", "jane", "jane.doe@example.invalid")],
                    ROSTER_FIXTURE, MAP_FIXTURE, set())
         == ([("prop-retro-1", "jane", "jane-doe")], [])),
        ("fix: no mapping is unfixable, never guessed",
         plan_fixes([("prop-retro-9", "bob", "bob@example.invalid")],
                    ROSTER_FIXTURE, MAP_FIXTURE, set())[0] == []),
        ("fix: a mapped handle outside the roster is unfixable",
         plan_fixes([("prop-retro-8", "sam", "sam@example.invalid")],
                    ROSTER_FIXTURE, MAP_FIXTURE, set())[0] == []),
        ("fix: a collision with another row is unfixable",
         plan_fixes([("prop-retro-1", "jane", "jane.doe@example.invalid")],
                    ROSTER_FIXTURE, MAP_FIXTURE, {"jane-doe"})[0] == []),
        ("fix: two rows mapped to one handle plan only the first",
         plan_fixes([("prop-retro-1", "jane", "jane.doe@example.invalid"),
                     ("prop-retro-2", "jd", "jane.doe@example.invalid")],
                    ROSTER_FIXTURE, MAP_FIXTURE, set())
         == ([("prop-retro-1", "jane", "jane-doe")],
             [("prop-retro-2", "jd", "jane.doe@example.invalid",
               "mapped handle 'jane-doe' is already on another proposal row")])),
    ]
    ok = True
    for name, good in cases:
        ok = ok and good
        print(f"  {'ok  ' if good else 'FAIL'} {name}")
    print(f"self-test: {'guard proven red then green' if ok else 'FAILURES ABOVE'}")
    return 0 if ok else 1


# --------------------------------------------------------------------- main

def open_db(path):
    con = sqlite3.connect(path)
    cols = {r[1] for r in con.execute("PRAGMA table_info(onboard_proposals)")}
    if "retroactive" not in cols:
        sys.exit("onboard_proposals has no `retroactive` column — deploy + start the new console build first")
    return con


def run_check(con, roster):
    rows = list(con.execute(
        "SELECT id, handle, email, retroactive FROM onboard_proposals WHERE status = 'enrolled'"))
    retro, normal = classify_enrolled(rows, roster)
    print(f"enrolled rows: {len(rows)}  handle not in roster: {len(retro) + len(normal)}")
    for id_, handle, email in retro:
        print(f"  FAIL {id_}  handle {handle!r}  {email}  — retroactive row; this handle never existed")
    for id_, handle, email in normal:
        print(f"  note {id_}  handle {handle!r}  {email}  — not retroactive; the identity may have left after enrolling")
    if retro:
        print(f"check: {len(retro)} retroactive enrolled row(s) name a handle the roster does not know (#1244)")
        return 1
    print("check: every retroactive enrolled row resolves in the roster")
    return 0


def run_fix(con, roster, handle_map, apply):
    rows = [(id_, handle, email) for id_, handle, email in con.execute(
        "SELECT id, handle, email FROM onboard_proposals "
        "WHERE retroactive = 1 AND id LIKE 'prop-retro-%'")
        if handle not in roster]
    taken = {h for (h,) in con.execute("SELECT handle FROM onboard_proposals") if h}
    updates, unfixable = plan_fixes(rows, roster, handle_map, taken)
    for id_, old, new in updates:
        print(f"  {id_}: handle {old!r} -> {new!r}")
    for id_, old, email, why in unfixable:
        print(f"  UNFIXABLE {id_}  handle {old!r}  {email}  — {why}")
    if not apply:
        print(f"dry-run — {len(updates)} row(s) would change; re-run with --apply to write")
        return 1 if unfixable else 0
    for id_, old, new in updates:
        cur = con.execute(
            "UPDATE onboard_proposals SET handle = ? WHERE id = ? AND handle = ? AND retroactive = 1",
            (new, id_, old))
        if cur.rowcount != 1:
            sys.exit(f"guarded update touched {cur.rowcount} rows for {id_} — db moved under us; nothing committed")
    con.commit()
    for id_, _, new in updates:
        got = con.execute("SELECT handle FROM onboard_proposals WHERE id = ?", (id_,)).fetchone()
        if not got or got[0] != new:
            sys.exit(f"read-back after commit disagrees for {id_}: {got!r}")
    print(f"wrote {len(updates)} handle fix(es), read back and confirmed")
    return 1 if unfixable else 0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--db")
    ap.add_argument("--kratos", default="http://127.0.0.1:4434")
    ap.add_argument("--env")
    ap.add_argument("--apply", action="store_true", help="write rows (default: dry-run)")
    ap.add_argument("--roster", metavar="FILE",
                    help="fabric handles, one per line — the authority every written handle is verified against")
    ap.add_argument("--handle-map", metavar="FILE",
                    help="lines of `email,handle` for identities whose email does not derive their handle")
    ap.add_argument("--check", action="store_true",
                    help="verify enrolled rows against the roster; exit 1 on a retroactive miss")
    ap.add_argument("--fix-handles", action="store_true",
                    help="repair prop-retro-* handles from --handle-map (guarded, dry-run unless --apply)")
    ap.add_argument("--self-test", action="store_true", help="run the pure logic on inline fixtures")
    ap.add_argument("--exclude", action="append", default=[], metavar="EMAIL",
                    help="skip this email (repeatable). Use for identities that are NOT "
                         "actually onboarded (login-only accounts awaiting real onboarding) — "
                         "a retroactive `onboarded` row would BLOCK their future onboarding "
                         "via the email-dedup guards.")
    args = ap.parse_args()

    if sum((args.check, args.fix_handles, args.self_test)) > 1:
        ap.error("--check, --fix-handles and --self-test are one mode each")
    if args.self_test:
        sys.exit(self_test())
    if not args.roster:
        ap.error("--roster is required: a handle is never written unverified (#1244)")
    if not args.db:
        ap.error("--db is required")
    with open(args.roster, encoding="utf-8") as source_file:
        roster = load_roster(source_file.read())
    if not roster:
        sys.exit(f"{args.roster} holds no handles — refusing to treat an empty roster as an authority")
    if args.handle_map:
        with open(args.handle_map, encoding="utf-8") as source_file:
            handle_map = load_handle_map(source_file.read())
    else:
        handle_map = {}

    con = open_db(args.db)
    if args.check:
        sys.exit(run_check(con, roster))
    if args.fix_handles:
        sys.exit(run_fix(con, roster, handle_map, args.apply))

    if not args.env:
        ap.error("--env is required for a backfill")
    excludes = {e.strip().lower() for e in args.exclude}
    ids = fetch_identities(args.kratos)
    existing = {r[0].strip().lower() for r in con.execute(
        "SELECT email FROM onboard_proposals") if r[0]}

    todo, refused, skipped = plan_backfill(ids, existing, excludes, roster, handle_map, args.env)
    print(f"identities: {len(ids)}  already-recorded/skipped: {skipped}  "
          f"to-backfill: {len(todo)}  refused: {len(refused)}")
    for row in todo:
        print(f"  {row[6]}  (handle {row[5]}, created {row[1]})")
    for email, cand in refused:
        print(f"  REFUSED {email}  — candidate handle {cand!r} is not in the roster; "
              f"add `{email},<real-handle>` to --handle-map")
    if not args.apply:
        print("dry-run — re-run with --apply to write")
        sys.exit(1 if refused else 0)
    con.executemany(
        """INSERT OR IGNORE INTO onboard_proposals
           (id, created_at, sponsor, sponsor_name, env, handle, email, status,
            manifest_json, lint_json, decided_by, decided_at, note,
            escalated, complete, petitions, tier, retroactive)
           VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)""", todo)
    con.commit()
    print(f"wrote {len(todo)} retroactive rows")
    sys.exit(1 if refused else 0)


if __name__ == "__main__":
    main()
