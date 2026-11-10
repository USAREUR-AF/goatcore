#!/usr/bin/env python3
"""
Did this peer's group membership change after it was minted, and who changed it?

ADR 1116 §9 makes the setup key the enforcement point: a node class picks the
groups and the key carries them. That holds right up until somebody edits
membership afterwards through the mgmt API, which every existing check is blind
to — the peer stays "covered", its shape stays legal, and the audit trail is one
opaque uuid in NetBird's activity log.

A peer minted by a correctly-scoped key can be widened into the INFRA group hours
or days later, and nothing else reports it. This check names it the same day.

HOW THE BASELINE IS ESTABLISHED. NetBird's activity log records a `Peer added`
event whose initiator is the SETUP KEY id. That key's `auto_groups` is the
membership the mint intended. Anything a peer holds beyond it was added later,
and the log usually says by whom and when.

Peers with no key baseline — enrolled by an interactive user login, or whose key
has since been deleted — are reported separately as UNKNOWN BASELINE. They are
not drift; they are peers this check cannot speak about, and saying so is the
point. A check that silently treats "cannot tell" as "fine" is the failure mode
this whole exercise keeps finding.

Declared deviations: ops/ci/membership-declared-deviations.yaml. A legitimate
later change is recorded there with who decided it and why, and prints as NOTE.
It is not a suppression list — a row with no reason is a bug in that file.

THE SECOND AXIS — `absent:`. The mint baseline catches groups a key never granted.
It is BLIND to a group the key DID grant that was later removed on purpose and
then put back, because "held" once again equals "granted" and nothing looks wrong.

So a removal is declarable in its own right: an `absent:` group on a peer
means "this was deliberately taken away, flag it if it returns". Held ∩ absent
prints as REVERTED and is strict-fatal. A deviations row may carry `group:`
(additions that are fine), `absent:` (removals that must stay removed), or both.

Usage:
  NB_API_URL=https://netbird.<zone> NB_API_TOKEN=... \\
    python3 ops/ci/check-membership-reconcile.py [--strict|--strict-reverted]
                                                [--metrics <path>]

Exit 0 unless a strict flag is set. --strict fails on either axis; the schedule
uses --strict-reverted, which fails ONLY on a reverted removal, so the standing
drift backlog does not paint it red on day one.

WHERE THIS RUNS. On the goatnet's own mgmt host, as the systemd timer installed
by ops/ansible/roles/mesh-membership-reconcile — NOT in GitHub Actions. It could
have run there (the NetBird mgmt API is the public enrollment endpoint), but the
token has to be admin-scoped, NetBird has no read-only role that can list setup
keys, and repo secrets are readable by any workflow on a same-repo branch. That
would have converted push access into mesh admin on a production goatnet. The
mgmt host already holds a token for this API, so running it there adds no new
credential exposure. A guard nobody runs is indistinguishable from no guard —
that is the other half of F-332 — but the fix for it does not get to be worse
than the finding.

--metrics writes a Prometheus textfile so the run is visible on the normal
metrics surface rather than only in one host's journal. A background job whose
age you cannot see is a future RCA with no evidence in it, so
`mesh_membership_reconcile_last_run_timestamp_seconds` is the load-bearing
series: it is stamped only after all four API reads succeeded, so a timer that
died, a host that lost its token, and a mesh that stopped answering all show up
the same honest way — as a number that stops moving.
"""

import json
import os
import sys
import time
import urllib.request

API = os.environ.get("NB_API_URL", "").rstrip("/")
TOKEN = os.environ.get("NB_API_TOKEN", "")
STRICT = "--strict" in sys.argv
# --strict-reverted is the schedulable gate. --strict also fails on the drift
# backlog: a job that is red on its first run is a job people learn to ignore.
# The reverted axis is empty by construction — every entry is a decision somebody
# wrote down — so it can gate a nightly from day one.
STRICT_REVERTED = "--strict-reverted" in sys.argv
METRICS = ""
if "--metrics" in sys.argv:
    i = sys.argv.index("--metrics")
    if i + 1 >= len(sys.argv):
        print("FAIL: --metrics needs a path", file=sys.stderr)
        sys.exit(2)
    METRICS = sys.argv[i + 1]
# THE THIRD AXIS — an invitation nobody accepted.
#
# The two axes above walk peers and ask what their minting key granted. Nothing
# there walks the other way: a key that minted NO peer is invisible to both,
# because there is no peer to iterate. A person whose key is never used may believe
# they are on the mesh; there is no failed enrollment and no rejected peer, so the
# only signal is an absence, and this axis watches for absences.
#
# Days before an unused key is worth printing. NOT zero: every key is legitimately
# unused between the mint and the person sitting down at their machine, and a
# check that flags that is a check people learn to scroll past.
# Parsed defensively: this is rendered from a group_var, and a typo there would
# otherwise take the whole daily audit down with a ValueError traceback BEFORE
# it reads the mesh — turning a threshold mistake into an outage of both other
# axes. Fall back and say so instead.
try:
    UNUSED_KEY_DAYS = int(os.environ.get("MEMBERSHIP_UNUSED_KEY_DAYS", "3"))
    if UNUSED_KEY_DAYS < 0:
        raise ValueError("negative")
except ValueError:
    print(f"WARNING: MEMBERSHIP_UNUSED_KEY_DAYS="
          f"{os.environ.get('MEMBERSHIP_UNUSED_KEY_DAYS')!r} is not a whole number of "
          f"days — using 3.", file=sys.stderr)
    UNUSED_KEY_DAYS = 3

DEV_FILE = os.environ.get(
    "MEMBERSHIP_DEVIATIONS",
    os.path.join(os.path.dirname(os.path.abspath(__file__)), "membership-declared-deviations.yaml"),
)

if not API or not TOKEN:
    print("FAIL: set NB_API_URL and NB_API_TOKEN", file=sys.stderr)
    sys.exit(2)


def get(path):
    req = urllib.request.Request(API + path, headers={"Authorization": f"Token {TOKEN}"})
    try:
        with urllib.request.urlopen(req, timeout=60) as r:
            return json.load(r)
    except Exception as e:
        # EXIT 2, NOT 1. 1 means "a declared removal came back" — the condition
        # this guard exists to catch. An unreadable mesh says NOTHING about
        # membership and must not be confusable with a finding. A traefik with
        # no ACME store serves its self-signed default cert; exit 1 would read as
        # a membership regression, and the deploy play's `exit: 2` guard would
        # not trip.
        print(f"FAIL: could not read {API}{path} — refusing to pass on an unread mesh\n      {e}",
              file=sys.stderr)
        sys.exit(2)


def load_deviations(path):
    """Tiny reader so this runs from any operator shell without PyYAML.

    Note this is deliberately NOT a YAML parse: a row repeats the `group:` /
    `absent:` key for several groups, which real YAML would collapse to the last
    one. The line reader keeps them all.
    """
    out, cur = {}, {}
    if not os.path.isfile(path):
        return out
    with open(path) as source_file:
        for line in source_file:
            s = line.strip()
            if s.startswith("- peer:"):
                if cur.get("peer"):
                    out.setdefault(cur["peer"], []).append(cur)
                cur = {"peer": s.split(":", 1)[1].strip(), "groups": [], "absent": []}
            elif s.startswith("- key:"):
                # A declared SETUP KEY (ADR 1158) — keyed "key:<name>" so it shares
                # this reader and cannot collide with a peer of the same name.
                if cur.get("peer"):
                    out.setdefault(cur["peer"], []).append(cur)
                cur = {"peer": "key:" + s.split(":", 1)[1].strip(),
                       "groups": [], "absent": []}
            elif s.startswith("group:") and cur:
                cur["groups"].append(s.split(":", 1)[1].strip())
            elif s.startswith("absent:") and cur:
                cur["absent"].append(s.split(":", 1)[1].strip())
            elif s.startswith("decided_by:") and cur:
                cur["by"] = s.split(":", 1)[1].strip()
            elif s.startswith("decided_on:") and cur:
                cur["on"] = s.split(":", 1)[1].strip()
    if cur.get("peer"):
        out.setdefault(cur["peer"], []).append(cur)
    return out


peers = get("/api/peers")
keys = get("/api/setup-keys")
events = get("/api/events")
groups = get("/api/groups")
declared = load_deviations(DEV_FILE)

gname = {g["id"]: g["name"] for g in groups}
key_by_id = {k["id"]: k for k in keys}

# peer id -> the setup key that minted it (initiator of its `Peer added` event)
minted_by, added_since = {}, {}
for e in events:
    act = e.get("activity")
    tgt = e.get("target_id")
    if act == "Peer added" and tgt:
        minted_by[tgt] = e.get("initiator_id")
    elif act == "Group added to peer" and tgt:
        m = e.get("meta") or {}
        added_since.setdefault(tgt, []).append(
            (e.get("timestamp", "")[:19], m.get("group", "?"), e.get("initiator_id", "?")[:8])
        )

drift, unknown, noted, reverted, with_baseline = [], [], [], [], 0
for p in peers:
    held = {g["name"] for g in (p.get("groups") or []) if g["name"] != "All"}

    # Deliberate removals that came back. Checked BEFORE the baseline branch on
    # purpose: this axis does not depend on the mint, so it still speaks about a
    # peer whose setup key was deleted.
    for d in declared.get(p["name"], []):
        back = sorted(held & set(d.get("absent") or []))
        if back:
            reverted.append((p["name"], back, d.get("on", "?"),
                             added_since.get(p["id"], [])))

    key = key_by_id.get(minted_by.get(p["id"], ""))
    if not key:
        unknown.append((p["name"], sorted(held)))
        continue
    with_baseline += 1
    granted = {gname.get(i, i) for i in (key.get("auto_groups") or [])}
    extra = held - granted
    if not extra:
        continue
    ok = {g for d in declared.get(p["name"], []) for g in d.get("groups", [])}
    undeclared = sorted(extra - ok)
    if extra & ok:
        noted.append((p["name"], sorted(extra & ok)))
    if undeclared:
        drift.append((p["name"], key["name"], sorted(granted), undeclared,
                      added_since.get(p["id"], [])))

# --- the fourth axis: keys that mint the WRONG RUNG (ADR 1158) --------
# The three axes above all ask "does this peer hold what its key granted". That
# is the right question for drift and the WRONG one for a key that grants the
# wrong thing: a peer corrected by hand while its key keeps minting the goatnet
# rung reads as clean here and rebuilds the defect on its next enrolment.
#
# So this axis asks the key directly: does it mint a MACHINE class at the
# goatnet rung when a site rung on this same server also offers that class?
# Advisory, never fatal — a host site's own gear legitimately sits at the
# goatnet rung (a break-glass peer, for example), so the
# honest output is a named list to confirm or declare, not an exit code.
#
# Declare an intended one with a `- key:` row in the deviations file, same shape
# as a `- peer:` row.
GOATNET_PREFIX = os.environ.get("MEMBERSHIP_GOATNET_PREFIX", "").strip().upper()

# ADR 1116 gear classes + the ADR 1092/1113/1114/1131/1216/1219 node classes. A MACHINE
# wears these; a person never does. Listed rather than derived because the
# role-vs-machine split is a fact about the vocabulary, not about the live data
# — DEVELOPER exists at both rungs too and belongs at neither exclusively.
MACHINE_SUFFIXES = {
    "KID", "EXTERNAL-KID", "FUSION", "CONTROL", "EXTERNAL-INTERMEDIARY",
    "SENSOR", "SENSOR-FEED", "SENSOR-TASKING", "SENSOR-TELEMETRY", "SENSOR-ARBITER",
    "EFFECTOR", "EFFECTOR-CUE", "EFFECTOR-TRACK", "EFFECTOR-TELEMETRY",
    "EFFECTOR-COMMAND", "EFFECTOR-ARBITER",
    "POD", "MOON", "NETWORK", "NETWORK-JUMP", "BREAKGLASS", "EDGE-SERVER", "PEERING-EDGE",
}

wrong_rung = []
if GOATNET_PREFIX:
    all_names = {g["name"] for g in groups}
    # Suffixes this goatnet declares, longest-match so EFFECTOR-CUE never reads
    # as prefix "<GOATNET>-EFFECTOR" + suffix "CUE".
    own = {n[len(GOATNET_PREFIX) + 1:] for n in all_names
           if n.startswith(GOATNET_PREFIX + "-")}
    # Which suffixes some OTHER prefix on this server also offers — i.e. where
    # a site rung exists for the machine to go to instead.
    elsewhere = {}
    for n in all_names:
        if n.startswith(GOATNET_PREFIX + "-"):
            continue
        for s in own:
            if n.endswith("-" + s) and len(n) > len(s) + 1:
                elsewhere.setdefault(s, set()).add(n[: -(len(s) + 1)])

    declared_keys = {k for k in declared if k.startswith("key:")}
    for k in keys:
        if k.get("revoked"):
            continue
        granted = sorted(gname.get(i, i) for i in (k.get("auto_groups") or []))
        at_goatnet = [g for g in granted
                      if g.startswith(GOATNET_PREFIX + "-")
                      and g[len(GOATNET_PREFIX) + 1:] in MACHINE_SUFFIXES
                      and g[len(GOATNET_PREFIX) + 1:] in elsewhere]
        if not at_goatnet:
            continue
        if ("key:" + k["name"]) in declared_keys:
            continue
        wrong_rung.append((
            # Sort LIVE keys first: an expired key is a record of how a peer got
            # its groups, a valid one is the defect still able to happen.
            0 if k.get("valid") else 1,
            k["name"],
            at_goatnet,
            sorted({p for g in at_goatnet
                    for p in elsewhere[g[len(GOATNET_PREFIX) + 1:]]}),
            bool(k.get("valid")),
            (k.get("last_used") or "").startswith("0001"),
        ))
    wrong_rung.sort()
wrong_rung_live = [r for r in wrong_rung if r[4]]

# --- the third axis: keys that minted nobody ---------------------------------
# `Setup key created` carries the key id as target_id and the minter as
# initiator_name, so the age and the "who invited them" both come from the
# events feed already read above — no extra call, no extra credential.
created_at, created_by = {}, {}
for e in events:
    if e.get("activity") == "Setup key created" and e.get("target_id"):
        created_at[e["target_id"]] = (e.get("timestamp") or "")[:19]
        created_by[e["target_id"]] = e.get("initiator_name") or e.get("initiator_email") or "?"


def age_days(stamp):
    """Whole days since an RFC3339 stamp, or None if it cannot be read.

    Sliced to 19 chars by the caller because NetBird emits nanosecond precision,
    which datetime.fromisoformat rejects on the Python that ships with 22.04.
    """
    if not stamp:
        return None
    try:
        return int((time.time() - time.mktime(
            time.strptime(stamp, "%Y-%m-%dT%H:%M:%S"))) // 86400)
    except (ValueError, OverflowError):
        return None


# `valid` excludes revoked and expired: those are closed invitations, and a
# closed invitation nobody used is history, not a live gap. usage_limit > 0
# excludes the unbounded infra keys (mesh-dns, relay bootstrap), which are
# SUPPOSED to sit idle — a bounded key is an invitation to a named set of
# devices, and that is the thing that can go unanswered.
unused, undated = [], []
for k in keys:
    if k.get("used_times") or not k.get("valid") or not (k.get("usage_limit") or 0) > 0:
        continue
    kid = k.get("id", "")
    age = age_days(created_at.get(kid, ""))
    if age is None:
        # The events feed is bounded, so an old key can outlive its own creation
        # event. Say so rather than dropping it — "cannot tell" reported as
        # "fine" is the failure mode this whole check keeps finding.
        exp = (k.get("expires") or "")[:10]
        undated.append((k.get("name", "?"),
                        "never" if exp.startswith("0001") else exp))
    elif age >= UNUSED_KEY_DAYS:
        # NetBird renders "no expiry" as the zero time, which prints as a
        # 0001-01-01 date and reads like a data error rather than a policy.
        exp = (k.get("expires") or "")[:10]
        unused.append((k.get("name", "?"), age, created_at[kid][:10],
                       created_by.get(kid, "?"),
                       "never" if exp.startswith("0001") else exp))
unused.sort(key=lambda r: -r[1])

print(f"membership reconcile — {API}")
print(f"  {len(peers)} peers · {with_baseline} with a key baseline · "
      f"{len(unknown)} without · {len(drift)} with undeclared additions · "
      f"{len(reverted)} with a reverted removal · "
      f"{len(unused)} key(s) issued to nobody\n")

for name, back, on, when in reverted:
    print(f"REVERTED {name}: {', '.join(back)}")
    print(f"      declared absent on {on} — removed deliberately, and it is back")
    for ts, g, who in sorted(when):
        if g in back:
            print(f"        + {g}  re-added at {ts}  by {who}")
    print()

for name, gs in noted:
    print(f"NOTE {name}: {', '.join(gs)} — DECLARED, see {os.path.basename(DEV_FILE)}")
if noted:
    print()

for name, kname, granted, extra, when in drift:
    print(f"DRIFT {name}")
    print(f"      minted by  {kname}")
    print(f"      key granted {', '.join(granted) or '(nothing)'}")
    print(f"      also holds  {', '.join(extra)}")
    for ts, g, who in sorted(when):
        if g in extra:
            print(f"        + {g}  at {ts}  by {who}")
    print()

if unknown:
    print(f"UNKNOWN BASELINE ({len(unknown)}) — enrolled by a user login, or the key was deleted.")
    print("  Not drift; this check cannot speak about them, which is worth knowing.")
    for name, gs in unknown[:12]:
        print(f"    {name}: {', '.join(gs) or '(only All)'}")
    if len(unknown) > 12:
        print(f"    … and {len(unknown) - 12} more")
    print()

if wrong_rung:
    print(f"KEYS MINTING THE WRONG RUNG ({len(wrong_rung)}, of which "
          f"{len(wrong_rung_live)} still VALID) — ADR 1158.")
    print("  Each grants a MACHINE class at the goatnet rung while a site rung on this")
    print("  same server offers the same class. Correcting the peer does not correct")
    print("  this: the next enrolment mints the old shape again.")
    print("  A VALID key is the defect still able to happen. An expired one is only the")
    print("  record of how a peer got its groups — leave it, re-mint the peer's next key.")
    for _, name, at, sites, valid, never_used in wrong_rung:
        state = "VALID" if valid else "expired"
        tail = ", never used" if never_used else ""
        print(f"    [{state}{tail}] {name}")
        print(f"      grants  {', '.join(at)}")
        print(f"      site rung available: {', '.join(sites)}")
    print("  Fix the KEY, then move the peer — that order, or the move is undone.")
    print("  A host site's own gear is legitimately here (a host-site BREAKGLASS is not")
    print("  a forward node): declare those with a `- key:` row in the deviations file.")
    print()
elif not GOATNET_PREFIX:
    print("NOTE: key-rung check skipped — MEMBERSHIP_GOATNET_PREFIX is unset, so this")
    print("  run cannot tell a goatnet-rung group from a site-rung one (ADR 1158).")
    print()

if unused:
    print(f"UNUSED KEYS ({len(unused)}) — issued, still valid, and no device has ever "
          f"presented them.")
    print("  Each one is a person who was invited and has not arrived. There is no failed")
    print("  enrollment to find and no peer to inspect: an unused key produces no signal at")
    print("  all, which is why this is counted rather than waited for. F-378.")
    for name, age, born, who, exp in unused:
        print(f"    {name}")
        print(f"      issued {born} by {who} · unused for {age} days · expires {exp}")
    print()

if undated:
    print(f"UNDATED UNUSED KEYS ({len(undated)}) — unused, but their creation event has")
    print("  aged out of the activity feed, so this check cannot say how long. Not a")
    print("  finding; a limit of the feed, stated rather than hidden.")
    for name, exp in undated[:8]:
        print(f"    {name} (expires {exp})")
    if len(undated) > 8:
        print(f"    … and {len(undated) - 8} more")
    print()

if drift:
    print("Every peer above holds groups its setup key never granted. Either the change")
    print("was intended — declare it — or the mint was bypassed. ADR 1116 §9.")
if reverted:
    print("A group listed above was taken away on purpose and is present again. Somebody")
    print("either undid a decision without recording it, or the decision was wrong and the")
    print("declaration should change. Both are edits — neither is silence. F-332.")
if unused:
    print("An unused key is a backlog row, not a regression — it does not fail this run.")
    print("Chase the person or revoke the key; either closes it. Leaving it open is the")
    print("state F-378 measures the cost of.")
if not drift and not reverted and not unused:
    print("OK: no undeclared additions beyond what each setup key granted, no declared")
    print("    removal has come back, and every issued key has been used.")
elif not drift and not reverted:
    print("OK on both membership axes: no undeclared additions and no reverted removal.")

if METRICS:
    # Written BEFORE the strict exits on purpose: the run that fails is exactly
    # the run whose numbers you want on the dashboard. Written atomically,
    # because node_exporter reads this directory on its own schedule and a
    # half-written file is a parse error that hides every series in it.
    host = API.split("//", 1)[-1].split("/", 1)[0].replace('"', "")
    now = int(time.time())
    body = f"""# HELP mesh_membership_reconcile_last_run_timestamp_seconds When the reconcile last ran.
# TYPE mesh_membership_reconcile_last_run_timestamp_seconds gauge
mesh_membership_reconcile_last_run_timestamp_seconds{{api="{host}"}} {now}
# HELP mesh_membership_peers_total Peers read from the management API.
# TYPE mesh_membership_peers_total gauge
mesh_membership_peers_total{{api="{host}"}} {len(peers)}
# HELP mesh_membership_unknown_baseline_total Peers this check cannot speak about (no surviving setup key).
# TYPE mesh_membership_unknown_baseline_total gauge
mesh_membership_unknown_baseline_total{{api="{host}"}} {len(unknown)}
# HELP mesh_membership_drift_total Peers holding groups their setup key never granted and undeclared.
# TYPE mesh_membership_drift_total gauge
mesh_membership_drift_total{{api="{host}"}} {len(drift)}
# HELP mesh_membership_reverted_total Declared-absent groups that are present again. Expected to be 0.
# TYPE mesh_membership_reverted_total gauge
mesh_membership_reverted_total{{api="{host}"}} {len(reverted)}
# HELP mesh_membership_unused_keys_total Valid bounded setup keys never presented by any device, older than the threshold.
# TYPE mesh_membership_unused_keys_total gauge
mesh_membership_unused_keys_total{{api="{host}"}} {len(unused)}
# HELP mesh_membership_unused_key_max_age_days Days since the oldest unused key was issued. 0 when there are none.
# TYPE mesh_membership_unused_key_max_age_days gauge
mesh_membership_unused_key_max_age_days{{api="{host}"}} {unused[0][1] if unused else 0}
# HELP mesh_membership_unused_keys_undated_total Unused keys whose creation event has aged out of the activity feed.
# TYPE mesh_membership_unused_keys_undated_total gauge
mesh_membership_unused_keys_undated_total{{api="{host}"}} {len(undated)}
# HELP mesh_membership_wrong_rung_keys_total Live setup keys minting a machine class at the goatnet rung where a site rung offers it (ADR 1158). Undeclared only.
# TYPE mesh_membership_wrong_rung_keys_total gauge
mesh_membership_wrong_rung_keys_total{{api="{host}"}} {len(wrong_rung)}
# HELP mesh_membership_wrong_rung_keys_valid Of the above, keys still VALID — the ones that can still mint the wrong rung. Alert on this, not the total.
# TYPE mesh_membership_wrong_rung_keys_valid gauge
mesh_membership_wrong_rung_keys_valid{{api="{host}"}} {len(wrong_rung_live)}
"""
    try:
        tmp = METRICS + ".tmp"
        with open(tmp, "w") as f:
            f.write(body)
        os.replace(tmp, METRICS)
    except OSError as e:
        # Never let the metrics sink fail the audit — but say so, loudly, rather
        # than leaving a stale file to be read as a fresh result.
        print(f"WARNING: could not write metrics to {METRICS}: {e}", file=sys.stderr)

if reverted and (STRICT or STRICT_REVERTED):
    sys.exit(1)
if drift and STRICT:
    sys.exit(1)
