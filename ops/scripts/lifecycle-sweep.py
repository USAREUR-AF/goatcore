#!/usr/bin/env python3
"""Dormancy + never-enrolled sweep — ADR 1136.

REPORT-ONLY BY DEFAULT. --apply is required to change anything, and each lane
must be named explicitly; there is no "apply everything".

Five lanes, from ADR 1136 D2/D2a:

  purge       key issued, NO peer ever created, key older than the grace period
              -> revoke the unredeemed key, delete the proposal record
  quarantine  people 14d inactive / gear 30d inactive  -> suspend membership
  revoke      people 60d offline  / gear 120d offline  -> ADR 1031 reversible revoke

EXEMPT (ADR 1216): peers in a NETWORK, CONTROL or EDGE-SERVER group are reported
when dormant but no lane ever quarantines or revokes them; an operator revokes
one explicitly.

WHY THE CLASS SPLIT. A person who has not connected in 60 days has left or
changed machine, and re-onboarding them is a portal flow that takes minutes.
A sensor can be legitimately dark for a mission and re-onboarding it needs
somebody to REACH it. Gear therefore gets four months rather than two, and a
peer whose class cannot be determined is treated AS GEAR — fail toward the
longer clock and the reversible action. The cost of wrongly sparing a laptop is
one stale record; the cost of wrongly revoking a deployed sensor is a truck roll.

WHY PURGE IS DIFFERENT FROM EVERYTHING ELSE HERE. It is the only irreversible
lane, and it is not a dormancy case at all: there is no peer, so no inactivity
clock exists and every threshold above is undefined. What exists is a live valid
credential issued to somebody who never used it, plus their name and email held
against an onboarding that never happened. Two guards, both load-bearing:
  - only where NO peer has EVER existed for that handle (not "no peer now" — a
    handle that enrolled and was later revoked has history and belongs in the
    dormancy lanes);
  - a grace period, so somebody onboarded this week is not purged before they
    have had a chance to act.

ON A SCOPE WARRANT GOVERNS (docs/design/mesh-change-control.md §4.5) — one whose
membership declaration DIRECTORY enforces, found at $LIFECYCLE_MEMBERSHIP_DIR
(or $LIFECYCLE_MEMBERSHIP_FILE, its older name), else
/etc/goat/warrant/membership/<GOATNET>/ (installed by the warrant-membership
role), else ops/warrant/membership/<GOATNET>/ in the checkout this runs from —
the quarantine and release lanes write NOTHING live. `--apply --lane
quarantine` prints each dormancy hold as an `exact: [<P>-PROBATION-<ROLE>]`
entry (to stdout, or --entries-out FILE) for a pull request that adds it to
that scope's holds.yaml — or, for a peer that already has an entry, replaces
it in the fragment that holds it (the output names which). It is a narrowing,
so it merges without waiting, and the membership converger applies it.
Release is deleting the entry. Revoke and purge are events, not standing
membership, and are unchanged.

usage:
  lifecycle-sweep.py                          # report every lane, change nothing
  lifecycle-sweep.py --json
  lifecycle-sweep.py --apply --lane quarantine --actor <handle>
  lifecycle-sweep.py --apply --lane purge --grace-days 30 --actor <handle>
  lifecycle-sweep.py --apply --lane release --peer <name-or-id> --actor <handle>
  lifecycle-sweep.py --apply --lane quarantine --actor <handle> --entries-out holds.yaml   # governed scope

Run ON the mgmt host.
"""
import argparse
import calendar
from contextlib import contextmanager
import json
import os
import re
import subprocess
import sys
import tempfile
import time
import urllib.request
import fcntl

# No built-in goatnet: the management API and the goatnet's name come from the
# environment. The peerview role's unit carries both in its EnvironmentFile
# (/etc/goat/lifecycle-sweep.env) and the onboard runner sets both on the
# command it runs; a hand-run names them the same way.
API = os.environ.get("NB_API", "")
SITE = os.environ.get("GOATNET", "")

# ADR 1136 D2. Days.
PERSON_QUARANTINE, PERSON_REVOKE = 14, 60
GEAR_QUARANTINE, GEAR_REVOKE = 30, 120
PURGE_GRACE_DEFAULT = 30
# This tool's OWN ledger — deliberately not the Rosenpass script's. Sharing one
# file would interleave two unrelated lifecycles and make the release lane
# replay rows it does not own.
LEDGER = os.environ.get("LIFECYCLE_LEDGER",
                        "/var/lib/peerview/lifecycle-sweep.log")
# Hold record file path, declared once by the peerview role and handed in as
# LIFECYCLE_HOLD_RECORDS (ops/lifecycle/README.md).
HOLD_RECORDS = os.environ.get("LIFECYCLE_HOLD_RECORDS", "")

# CLASSIFICATION IS BY GROUP MEMBERSHIP, WHICH IS A PROXY — SEE THE LIMIT BELOW.
#
# ADR 1136 D3 says class is read from `node_class` in the registry. There is no
# node_class group on this goatnet: the live vocabulary is ROLE-based
# (<SITE>-DEVELOPER, <SITE>-OBSERVER, <SITE>-PLATFORM-ADMIN) plus a handful of
# infrastructure groups. So this infers class from the two families instead, and
# that inference is the weakest part of this tool. It is stated rather than
# buried because it decides which clock a peer is on.
#
# Never from the hostname: a device called `Mac` tells you nothing, and this
# fleet has peers whose names carry no attribution at all.
GEAR_GROUPS = {"SENSOR", "EFFECTOR", "GEAR", "INFRA", "NETWORK", "POD", "BEARER",
               "ROUTER", "DNS", "MONITOR", "MOON", "RELAY", "BRIDGE"}
# A human role grant means a human's device. These are grants to PEOPLE, so a
# peer holding one is somebody's laptop or phone.
PERSON_ROLE_GROUPS = {"DEVELOPER", "OBSERVER", "PLATFORM-ADMIN", "ADMIN",
                      "PARTNER", "OPERATOR", "ANALYST",
                      # ADR 1216: a person group, never gear. A laptop holding
                      # only <SCOPE>-NET-ENGINEER is on the person clock.
                      "NET-ENGINEER"}

NAMED = re.compile(r"^peer-(.+?)-[a-z0-9-]+-\d{4}-\d{2}-\d{2}")

# ADR 1216 — EXPLICIT REVOCATION ONLY. A switch, a C2 console or an edge server
# can sit dark for weeks between exercises, and losing its mesh membership means
# somebody travelling to the device to re-enroll it. Peers holding one of these
# classes are still REPORTED when dormant, in their own bucket, but no lane's
# --apply ever quarantines or revokes them: that takes an operator acting on the
# one device by name. Matched on the group SUFFIX, so <SCOPE>-NETWORK counts and
# <SCOPE>-NETWORK-JUMP does not.
#
# A quarantined device holds only its hold group: the quarantine moved its
# class group into the hold record's prior_groups. So the class is read from
# the live groups AND from the record, or a device quarantined before this
# rule existed would reach the revoke lane at 120 days with nothing to say
# it was a switch.
EXPLICIT_REVOCATION_ONLY = ("NETWORK", "CONTROL", "EDGE-SERVER")


def explicit_revocation_only(peer, record=None):
    """The ADR 1216 class this peer holds, live or in its hold record's
    prior_groups, that exempts it from the sweep; None when it holds none."""
    names = list(peer_group_names(peer))
    if record:
        names += [str(g) for g in (record.get("prior_groups") or [])]
    for g in names:
        name = g.upper()
        for suffix in EXPLICIT_REVOCATION_ONLY:
            if name == suffix or name.endswith("-" + suffix):
                return suffix
    return None

# ── warrant-governed scopes ────────────────────────────────────────────────
MEMBERSHIP_ETC_DIR = "/etc/goat/warrant/membership"
MEMBERSHIP_REPO_DIR = os.path.normpath(os.path.join(
    os.path.dirname(os.path.abspath(__file__)), "..", "warrant", "membership"))


def membership_file(site=None):
    """The membership declaration (a scope DIRECTORY) that makes this scope
    governed, or None.

    Governed = the directory exists and its _scope.yaml does not say
    `converge: report`; a missing or unreadable _scope.yaml fails toward
    governed, as the goat CLI does. A host still carrying the retired single
    file <site>.yaml (a role not yet redeployed) is read the same way, so a
    stale install never turns the sweep back into a live writer.

    The peerview role copies this script alone onto the host, so it cannot
    import ops/warrant/membership.py; the lookup and the entry rendering below
    are the small part of it this lane needs, in the same shape.
    """
    site = site or SITE
    explicit = os.environ.get("LIFECYCLE_MEMBERSHIP_DIR", "") or os.environ.get("LIFECYCLE_MEMBERSHIP_FILE", "")
    candidates = [explicit] if explicit else [
        p for d in (MEMBERSHIP_ETC_DIR, MEMBERSHIP_REPO_DIR)
        for p in (os.path.join(d, site), os.path.join(d, "%s.yaml" % site))]
    for path in candidates:
        if os.path.isdir(path):
            return path if _converge_enforce(os.path.join(path, "_scope.yaml")) else None
        if os.path.isfile(path):
            return path if _converge_enforce(path) else None
    return None


def _converge_enforce(path):
    """Governed means not `converge: report`; a `report` scope is an adoption draft."""
    import re as _re
    try:
        with open(path) as fh:
            for line in fh:
                m = _re.match(r"^converge:\s*([a-z]+)", line)
                if m:
                    return m.group(1) != "report"
    except OSError:
        pass  # unreadable converge file
    return True   # unreadable: fail toward governed, as the goat CLI does


HOLDS_FRAGMENT = "holds.yaml"


def entry_fragments(scope_path):
    """peer name -> fragment (relative path) already declaring it, read from
    the installed scope directory. One entry per peer across fragments, so a
    hold for a declared peer replaces that entry where it is."""
    out = {}
    if not scope_path or not os.path.isdir(scope_path):
        return out
    pat = re.compile(r"^\s*-\s+peer:\s*[\"']?(?P<peer>[^\"'#\s]+)")
    for root, _dirs, files in os.walk(scope_path):
        for fn in sorted(files):
            if not fn.endswith(".yaml") or fn.startswith("_"):
                continue
            full = os.path.join(root, fn)
            try:
                with open(full) as fh:
                    for line in fh:
                        m = pat.match(line)
                        if m:
                            out.setdefault(m.group("peer"), os.path.relpath(full, scope_path))
            except OSError:
                continue
    return out


def hold_entry(row, hold_group_name, actor):
    """A dormancy hold as a warrant membership entry (mesh-change-control §4.5)."""
    threshold = PERSON_QUARANTINE if row.get("class") == "person" else GEAR_QUARANTINE
    days = row.get("days")
    return {
        "peer": row["name"],
        "exact": [hold_group_name],
        "owner": actor,
        "reason": "dormant %s (threshold %dd), lifecycle-sweep ADR 1136"
                  % (("%.1fd" % days) if days is not None else "?", threshold),
    }


def render_entries(entries):
    """YAML list items in ops/warrant/membership.py's render_entry shape."""
    out = []
    for e in entries:
        out.append("  - peer: %s" % e["peer"])
        out.append("    exact: [%s]" % ", ".join(e["exact"]))
        out.append("    owner: %s" % e["owner"])
        out.append('    reason: "%s"' % e["reason"].replace('"', '\\"'))
    return "\n".join(out) + ("\n" if out else "")


# Same shape as NAMED, but capturing the MINT DATE the peer-name grammar stamps
# into every key name (`peer-<handle>-<goatnet>-YYYY-MM-DD`).
NAMED_MINTED = re.compile(r"^peer-.+?-[a-z0-9-]+-(\d{4})-(\d{2})-(\d{2})")


def key_age_days(k):
    """Age of a setup key in days, and which clock produced it.

    THE PURGE LANE ASKS: how long has this credential sat unredeemed? Only a
    CREATION time answers that, and NetBird's setup-key object does not carry
    one — the fields are `updated_at`, `last_used` and `expires`.

    This used to fall back to `updated_at`, which is a LAST-MODIFIED stamp: any
    touch (a bulk revoke, an auto_groups change from the generated ACL sync)
    rewrites it and restarts the grace clock. Across many unredeemed credentials
    the median `updated_at` age can be ZERO DAYS while most were actually minted
    months earlier, so a lane keyed on it reports almost nothing to do and reads as
    "nothing to clean up" — an empty lane
    that meant a broken clock rather than an empty world (see days_since below).

    The mint date is already in the key NAME, stamped by the peer-name grammar,
    so prefer it and fall back to `updated_at` only for a key whose name carries
    no date. The chosen clock is returned so the roster can say which one it
    used rather than presenting both as the same number.
    """
    m = NAMED_MINTED.match(k.get("name") or "")
    if m:
        minted = calendar.timegm((int(m.group(1)), int(m.group(2)), int(m.group(3)),
                                  0, 0, 0, 0, 0, 0))
        return (time.time() - minted) / 86400.0, "mint-date"

    # NO DATE IN THE NAME. `updated_at` is the clock this function exists to stop
    # using — any touch resets it — so falling back to it here just reopens the
    # bug for exactly the keys the grammar did not stamp. A hand-made key called
    # `peer-<handle>-<goatnet>-clean` does precisely that: it reads as fresh while
    # being months old and close to expiry, and it survives a pass it should not.
    #
    # `expires` is the only other creation-anchored fact NetBird gives us —
    # expiry is mint plus the TTL, so age = TTL - remaining. The TTL a key was
    # minted under is not recorded, so this cannot produce a number; what it CAN
    # produce is a true statement about whether the key is near the end of any
    # plausible TTL, which is what the grace check actually needs.
    #
    # Returns None with an explicit clock name rather than a guess. None is not
    # "young": every caller must treat it as UNAGEABLE and say so, which is the
    # honest shape and the one the empty-lane bugs in this file kept violating.
    return None, "unageable-no-date-in-name"


def sh(*a):
    return subprocess.run(a, capture_output=True, text=True).stdout


def container_env(name, key):
    # `sudo` only when not already root. The systemd unit runs as root under
    # NoNewPrivileges=true, where invoking a setuid binary is at best pointless
    # and at worst the thing that fails.
    pre = () if os.geteuid() == 0 else ("sudo",)
    for line in sh(*pre, "docker", "inspect", name,
                   "--format", "{{range .Config.Env}}{{println .}}{{end}}").splitlines():
        if line.startswith(key + "="):
            return line.split("=", 1)[1]
    return ""


def api(path, method="GET", body=None, token=""):
    if not API:
        raise SystemExit("NB_API is not set: the goatnet's management API base URL "
                         "(/etc/goat/lifecycle-sweep.env carries it on a mgmt host).")
    req = urllib.request.Request(API + path, method=method)
    req.add_header("Authorization", "Token " + token)
    req.add_header("Content-Type", "application/json")
    data = json.dumps(body).encode() if body is not None else None
    with urllib.request.urlopen(req, data, timeout=30) as r:
        raw = r.read()
        return json.loads(raw) if raw else {}


def psql(db, sql):
    fd, path = tempfile.mkstemp(suffix=".sql")
    os.write(fd, sql.encode())
    os.close(fd)
    os.chmod(path, 0o644)          # container postgres uid must read it
    sh("sudo", "docker", "cp", path, "postgres-a:/tmp/lcs.sql")
    # -A -t -F flags, never \pset in the file: psql echoes
    # `Field separator is "|".`, a line containing the separator itself.
    out = sh("sudo", "docker", "exec", "postgres-a", "psql", "-U", "postgres",
             "-d", db, "-A", "-t", "-F", "|", "-f", "/tmp/lcs.sql")
    os.unlink(path)
    return [r for r in (l.split("|") for l in out.splitlines() if "|" in l)
            if len(r) >= 2]


def days_since(ts):
    """Age in days of an API timestamp like '2026-07-24T12:11:21.985254Z'.

    Two bugs lived here and BOTH returned a wrong answer silently rather than
    raising, which is the dangerous shape for a function that decides whether to
    revoke somebody:

      - The format strings included a trailing 'Z' while the input is sliced to
        19 characters, which cuts the Z off. Every parse failed, every peer was
        skipped, and the sweep reported ZERO to quarantine and ZERO to revoke
        with 61 peers disconnected. An empty lane reads as "nothing to do".
      - time.mktime interprets the struct as LOCAL time while these timestamps
        are UTC, so every age was off by the host's offset. calendar.timegm is
        the UTC counterpart.
    """
    if not ts:
        return None
    for f in ("%Y-%m-%dT%H:%M:%S", "%Y-%m-%d %H:%M:%S"):
        try:
            return (time.time() - calendar.timegm(time.strptime(ts[:19], f))) / 86400.0
        except ValueError:
            continue
    return None


def classify(peer):
    """GEAR or PERSON. Unknown resolves to GEAR — ADR 1136 D3.

    Order matters: gear wins over a role grant, because an infrastructure host
    can also carry a role group and the consequence of getting it wrong is
    asymmetric. Wrongly sparing a laptop costs one stale record; wrongly
    revoking a deployed sensor costs a truck roll.

    An earlier version returned "person" for anything holding ANY group, which
    is backwards from D3 — it put every unclassifiable peer on the SHORT clock,
    the exact opposite of failing safe.
    """
    groups = {((g.get("name") if isinstance(g, dict) else str(g)) or "").upper()
              for g in (peer.get("groups") or [])}
    tails = {g.rsplit("-", 1)[-1] for g in groups}
    if (groups & GEAR_GROUPS) or (tails & GEAR_GROUPS):
        return "gear"
    if (groups & PERSON_ROLE_GROUPS) or (tails & PERSON_ROLE_GROUPS) or any(
            g.endswith("-" + role) for g in groups for role in PERSON_ROLE_GROUPS):
        return "person"
    return "gear"          # unknown -> the longer clock, never the shorter


def peer_group_names(peer):
    return [
        ((g.get("name") if isinstance(g, dict) else str(g)) or "")
        for g in (peer.get("groups") or [])
    ]


@contextmanager
def _hold_records_lock():
    """Hold the sidecar lock throughout a hold-record update.

    Writers serialize on an exclusive flock of a sidecar file
    (``HOLD_RECORDS + ".lock"``) so that a delete+replace and an append
    never interleave their read-modify-write sequence. The lock lives
    in the same directory as the record file and is never replaced.
    """

    if not HOLD_RECORDS:
        yield
        return
    lock_path = HOLD_RECORDS + ".lock"
    dirpath = os.path.dirname(lock_path) or "."
    os.makedirs(dirpath, exist_ok=True)
    with open(lock_path, "a") as fh:
        fcntl.flock(fh.fileno(), fcntl.LOCK_EX)
        try:
            yield
        finally:
            fcntl.flock(fh.fileno(), fcntl.LOCK_UN)


# Mirrors "required" in ops/lifecycle/schema/hold-record.v1.schema.json;
# the test suite pins the two together so the sweep and the console never
# disagree on what a bad line is.
HOLD_RECORD_REQUIRED = ("v", "lane", "site", "peer_id", "peer_name", "handle",
                        "entered_at", "hold_group", "prior_groups", "actor")


def _is_bad_hold_record(data):
    """A bad line is not a JSON object, not v1, lacks a required v1 field,
    or carries an empty peer_id (ops/lifecycle/README.md, "A bad line is
    skipped, never fatal, and always counted")."""
    if not isinstance(data, dict):
        return True
    if any(k not in data for k in HOLD_RECORD_REQUIRED):
        return True
    if data.get("v") != 1:
        return True
    if not str(data.get("peer_id") or "").strip():
        return True
    # Types, so a hand-edited or foreign-writer line cannot crash a reader:
    # a list where a string belongs is a bad line, not a TypeError.
    if not all(isinstance(data.get(k), str) for k in ("peer_id", "peer_name", "hold_group")):
        return True
    if not isinstance(data.get("prior_groups"), list) or not all(
            isinstance(g, str) for g in data["prior_groups"]):
        return True
    # Lane and group must agree (the schema's allOf). A hand-edited
    # `dormancy` line pointing at QUARANTINED would otherwise be released
    # without the attestation the fallback demands for that group.
    hg = str(data.get("hold_group") or "").upper()
    if data.get("lane") == "dormancy" and "-PROBATION-" not in hg:
        return True
    if data.get("lane") == "security" and not hg.endswith("-QUARANTINED"):
        return True
    if data.get("lane") not in ("dormancy", "security"):
        return True
    if "phase" in data and data.get("phase") != "releasing":
        # Same answer as the validator (schema enum), so the two readers
        # never disagree on whether a line is a record.
        return True
    return False


def _read_hold_lines():
    """Read HOLD_RECORDS under the caller's lock.

    Returns (rows, malformed): rows are (raw_line, parsed_dict) for every
    well-formed v1 line; malformed are the raw bad lines, skipped and
    counted, never fatal. Blank lines are ignored and not counted. Writers
    carry malformed lines through unchanged rather than dropping them so
    validate-hold-records.py can name them; a bad line never aborts a
    release or a revoke, and never acts (its peer has no record).
    """
    rows, malformed = [], []
    try:
        # errors="replace": a corrupt byte makes that line a bad line (skipped,
        # counted), never a decode error that kills the report-only run.
        with open(HOLD_RECORDS, "r", errors="replace") as fh:
            for raw in fh:
                if not raw.strip():
                    continue
                try:
                    data = json.loads(raw)
                except ValueError:
                    malformed.append(raw)
                    continue
                if _is_bad_hold_record(data):
                    malformed.append(raw)
                    continue
                rows.append((raw, data))
    except FileNotFoundError:
        pass  # no hold-records file yet
    return rows, malformed


def _warn_bad_hold_lines(malformed):
    """Report skipped bad lines on stderr; the contract says every reader
    reports how many it skipped."""
    if malformed:
        print("  WARNING %d bad hold-record line(s) in %s skipped (carried through "
              "unchanged); affected peers have no record. Run "
              "ops/lifecycle/validate-hold-records.py to name them."
              % (len(malformed), HOLD_RECORDS), file=sys.stderr)


def hold_records_summary(live_groups=None):
    """Count well-formed and bad lines for the report, plus the records whose
    peer is no longer on the mesh (deleted in the NetBird UI, or an old
    enrolment's id); None when the record file is not configured or does not
    exist yet."""
    if not HOLD_RECORDS:
        return None
    # No lock: writers replace the file atomically, so a reader always sees
    # a whole file, and taking the sidecar lock would need WRITE access to
    # the directory — a non-root hand run of the report would die on it.
    try:
        with open(HOLD_RECORDS):
            pass
        rows, malformed = _read_hold_lines()
    except FileNotFoundError:
        return None
    except OSError as exc:
        return {"path": HOLD_RECORDS, "records": None, "bad_lines_skipped": None,
                "error": "%s: %s" % (type(exc).__name__, exc)}
    def brief(d):
        return {"peer_id": d.get("peer_id"), "peer_name": d.get("peer_name"),
                "hold_group": d.get("hold_group")}
    live_groups = live_groups if live_groups is not None else {}
    orphans = [brief(d) for _, d in rows if d.get("peer_id") not in live_groups]
    # A record whose live peer is NOT in its hold group: a hold that failed at
    # the add, or an out-of-band release. Named here so it does not vanish
    # once the peer reconnects (the candidate list only sees dormant peers).
    stale = [brief(d) for _, d in rows
             if d.get("peer_id") in live_groups
             and d.get("hold_group") not in live_groups[d.get("peer_id")]]
    releasing = len([1 for _, d in rows if d.get("phase") == "releasing"])
    return {"path": HOLD_RECORDS, "records": len(rows), "bad_lines_skipped": len(malformed),
            "orphans": orphans, "stale": stale, "releasing": releasing}


def _rewrite_hold_file(lines):
    """Temp-file + atomic replace under the caller's lock.

    Keeps the existing file's mode (the contract says every ADMIN can read
    the file; a fresh NamedTemporaryFile would silently narrow it to 0600),
    defaulting to 0644 when the file is new.
    """
    dirpath = os.path.dirname(HOLD_RECORDS) or "."
    os.makedirs(dirpath, exist_ok=True)
    try:
        existing = os.stat(HOLD_RECORDS)
        mode = existing.st_mode & 0o777
        gid = existing.st_gid
    except FileNotFoundError:
        mode = 0o644
        gid = -1
    tmp = tempfile.NamedTemporaryFile("w", dir=dirpath, delete=False)
    try:
        for raw in lines:
            tmp.write(raw if raw.endswith("\n") else raw + "\n")
        tmp.flush()
        os.fsync(tmp.fileno())
    finally:
        tmp.close()
    os.chmod(tmp.name, mode)
    if gid != -1:
        os.chown(tmp.name, -1, gid)
    os.replace(tmp.name, HOLD_RECORDS)


def _lookup_hold_record(peer_id, peer_name, live_ids=None, name_count=1):
    """Find the record for a peer: by id first, by name as a narrow fallback.

    The name fallback exists for ONE case: a peer re-enrolled under a new id
    while its old enrolment's record is still on file. It applies only when
    the name is unique among live peers (*name_count* == 1) AND the record's
    own peer_id is not a live peer (*live_ids*). Otherwise a record for a
    different live peer that happens to share the name (`Mac`) would be
    acted on, or deleted as stale, on behalf of the wrong peer.

    Returns the parsed record or None. The returned record's own `peer_id`
    is the key a caller must delete by; it may differ from *peer_id* when the
    match was by name.
    """
    if not HOLD_RECORDS:
        return None
    rows, _ = _read_hold_lines()
    live_ids = set(live_ids or ())
    by_name = None
    for _, data in rows:
        if data.get("peer_id") == peer_id:
            return data
        if (by_name is None and peer_name and name_count == 1
                and data.get("peer_name") == peer_name
                and data.get("peer_id") not in live_ids):
            by_name = data
    return by_name


def hold_groups_of(names):
    """The hold groups among *names* (upper-cased): any `-PROBATION-` group
    and any `-QUARANTINED` group."""
    return [n for n in names if "-PROBATION-" in n or n.endswith("-QUARANTINED")]


def _group_suffix(name: str) -> str:
    name = (name or "").upper()
    return name.rsplit("-", 1)[-1]


def _role_suffix(name: str) -> str:
    """Canonical role suffix for a group name.

    Role groups are of the form `<SITE>-<ROLE>` where `<ROLE>` can itself
    contain hyphens (e.g. `PLATFORM-ADMIN`). `_group_suffix()` only returns
    the last component, which mis-classifies `SITE-PLATFORM-ADMIN` as
    `ADMIN`. Use the current `SITE` prefix when present and return the
    full role code after it; fall back to the last component for
    non-site-prefixed names.
    """

    name = (name or "").upper()
    site_prefix = SITE.upper() + "-"
    if name.startswith(site_prefix):
        return name[len(site_prefix) :]
    return name.rsplit("-", 1)[-1]


def choose_hold_group(peer, groups_by_name):
    """Select the dormancy hold group for a peer.

    Rule (GH #1040):
      * exactly one role group with a PROBATION- mirror -> that mirror;
      * a gear-class group -> <SITE>-PROBATION-GEAR;
      * anything else (two roles, no role, role with no mirror,
        contradictory hold+role) -> refuse.

    Returns (hold_group, reason, group_names_upper). hold_group is None on
    refusal or when the peer is already held; reason is None for "already
    held" (silent skip in the candidate list).
    """

    names = [n.upper() for n in peer_group_names(peer)]
    names_no_all = [n for n in names if n != "ALL"]

    held = hold_groups_of(names_no_all)

    canonical_roles = {
        "PLATFORM-ADMIN",
        "MONITOR",
        "DEVELOPER",
        "OBSERVER",
        "MOON",
        "POD",
    }
    role_groups = [
        n
        for n in names_no_all
        if _role_suffix(n) in canonical_roles
        and "-PROBATION-" not in n
        and not n.endswith("-QUARANTINED")
    ]

    if held:
        # A held peer holds its hold group and nothing else: the hold removes
        # EVERY prior group, role or not (SENSOR, INFRA, PARTNER ...). So any
        # other group beside a hold group is inconsistent live state — a
        # stranded hold, or an admin's addition — and so is more than one
        # hold group at once (PROBATION on top of QUARANTINED, or the
        # reverse). Naming only the canonical roles here made a stranded
        # GEAR hold invisible and therefore never finished.
        others = [n for n in names_no_all if n not in held]
        if others:
            reason = (
                "in hold group(s) %s and group(s) %s"
                % (",".join(sorted(set(held))), ",".join(sorted(set(others))))
            )
            return None, reason, names_no_all
        if len(set(held)) > 1:
            reason = "in hold group(s) %s (more than one hold group)" % ",".join(sorted(set(held)))
            return None, reason, names_no_all
        # Already held, nothing else: skip silently from the quarantine
        # candidate list but keep eligible for revoke.
        return None, None, names_no_all

    role_with_mirror = []
    role_without_mirror = []
    for n in role_groups:
        suffix = _role_suffix(n)
        site_prefix = SITE.upper()
        mirror = "%s-PROBATION-%s" % (site_prefix, suffix)
        if mirror in groups_by_name:
            role_with_mirror.append(mirror)
        else:
            role_without_mirror.append(n)

    if role_without_mirror:
        reason = (
            "role group(s) %s have no PROBATION mirror"
            % ",".join(sorted(set(role_without_mirror)))
        )
        return None, reason, names_no_all
    if len(role_with_mirror) == 1:
        return role_with_mirror[0], None, names_no_all
    if len(role_with_mirror) > 1:
        reason = (
            "multiple role groups with mirrors %s"
            % ",".join(sorted(set(role_groups)))
        )
        return None, reason, names_no_all

    gear_groups = [
        n
        for n in names_no_all
        if _group_suffix(n) in GEAR_GROUPS
        and "-PROBATION-" not in n
        and not n.endswith("-QUARANTINED")
    ]
    if gear_groups:
        site_prefix = SITE.upper()
        hold = "%s-PROBATION-GEAR" % site_prefix
        if hold not in groups_by_name:
            reason = "no %s group on this goatnet" % hold
            return None, reason, names_no_all
        return hold, None, names_no_all

    reason = "no role group with mirror and no gear class group"
    return None, reason, names_no_all


INCONSISTENT_PREFIX = "in hold group(s)"


def is_inconsistent(reason):
    """True when a refusal reason names contradictory live state: the peer is
    in a hold group AND a role group. Every other refusal (two roles, no
    role, no mirror) describes a peer the sweep deliberately leaves in its
    full groups, which is the intended fail-closed outcome, not a fault."""
    return bool(reason) and reason.startswith(INCONSISTENT_PREFIX)


def _fmt_days(days):
    return "%6.1fd" % days if days is not None else "     -"


def _group_put_body(g, peers_ids):
    """NetBird's updateGroup resets any field the body omits; `resources`
    is unused on our goatnets today but is carried through so a group PUT
    from this tool never empties a field it did not mean to touch."""
    return {"name": g["name"], "peers": peers_ids,
            "resources": g.get("resources") or []}


def _peer_ids(g):
    return [p["id"] if isinstance(p, dict) else p for p in (g.get("peers") or [])]


def _group_set_member(group_id, peer_id, present, token):
    """Add (*present*) or remove *peer_id* in one group, re-reading the
    group IMMEDIATELY before the PUT. NetBird's group update is a whole-list
    replace with no If-Match, so a PUT built from a list read earlier in the
    run silently reverts whatever another writer (the console, an admin)
    did to that group in between — dropping THEIR held peer out of its hold
    group, or putting it back into its role group. Returns True when the
    PUT was sent, False when the group already had the wanted membership."""
    g = api("/api/groups/" + group_id, token=token)
    peers_ids = _peer_ids(g)
    if present == (peer_id in peers_ids):
        return False
    peers_ids = ([x for x in peers_ids if x != peer_id] if not present
                 else peers_ids + [peer_id])
    api("/api/groups/" + group_id, "PUT", _group_put_body(g, peers_ids), token=token)
    return True


def _remove_peer_from_groups(peer_id, group_ids, token):
    """Take *peer_id* out of every group in *group_ids* (ids), one re-read
    plus PUT per group that still lists it. Returns the number of groups
    changed. Shared by the hold (after the record is written) and by the
    resume of a stranded hold, so both remove the same way."""
    changed = 0
    for gid in group_ids:
        if gid and _group_set_member(gid, peer_id, False, token):
            changed += 1
    return changed


def _resumable_hold(peer, record):
    """A stranded hold is a peer in its record's hold group AND in some of
    its record's prior groups: the hold's add and record landed, the removal
    did not (a crash between the two). Returns the ids of the prior groups to
    finish removing, or None when the state is NOT a stranded hold: no
    record, record names another hold group, or the peer holds a group the
    record never listed (an admin added it since; hands, not the sweep)."""
    if not record:
        return None
    if record.get("lane") != "dormancy" or record.get("phase") == "releasing":
        # A security record is not this tool's to act on in either
        # direction; a release in progress has the same live shape as a
        # stranded hold and finishing "the hold" would undo the release.
        return None
    hold_group = record.get("hold_group") or ""
    prior = set(record.get("prior_groups") or [])
    leftover_ids, leftover_names = [], []
    in_hold_group = False
    for g in peer.get("groups") or []:
        name = (g.get("name") if isinstance(g, dict) else str(g)) or ""
        gid = g.get("id") if isinstance(g, dict) else g
        if name == "All":
            continue
        if name == hold_group:
            in_hold_group = True
            continue
        leftover_ids.append(gid)
        leftover_names.append(name)
    if not in_hold_group or not leftover_names:
        return None
    if not set(leftover_names) <= prior:
        return None
    return leftover_ids


def record_handle(name):
    """The handle for a hold record, or None when the name does not say.

    Grammar: `peer-<handle>[-<short_tag>]-<site>-<date>[-<host>]`, and BOTH
    handle and short_tag may contain hyphens. Without the registry the sweep
    cannot tell `jane-doe` + no tag from `jane` + tag `doe`, and the console
    matches records to members by this field — so a guess would attribute a
    hold to the wrong person. Recorded only when the part before the site
    has no hyphen (unambiguous); otherwise null, stated rather than invented.
    (`NAMED` stays first-segment for the purge lane, which compares key
    names to peer names parsed the same way and never attributes.)"""
    m = re.match(r"^peer-(.+?)-%s-\d{4}-\d{2}-\d{2}" % re.escape(SITE.lower()), (name or "").lower())
    if not m or "-" in m.group(1):
        return None
    return m.group(1)


def revoked_peer_ids():
    """Peer IDs the revoke lane has already recorded in LEDGER.

    Every revoke row is ``date\trevoke\tname\tid\tgroups``. A row with no ID
    in column 4 names no peer, so it marks nothing done. No ledger yet (a
    fresh host) means nothing is done.
    """
    ids = set()
    try:
        with open(LEDGER) as fh:
            for line in fh:
                f = line.rstrip("\n").split("\t")
                if len(f) >= 4 and f[1] == "revoke" and f[3]:
                    ids.add(f[3])
    except FileNotFoundError:
        pass  # no ledger file yet
    return ids


def append_hold_record(peer, hold_group, prior_groups, actor, days, lane="dormancy"):
    if not HOLD_RECORDS:
        return
    entered_at = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
    handle = record_handle(peer.get("name"))
    rec = {
        "v": 1,
        "lane": lane,
        "site": SITE.lower(),
        "peer_id": peer.get("id"),
        "peer_name": peer.get("name"),
        "handle": handle,
        "entered_at": entered_at,
        "hold_group": hold_group,
        "prior_groups": sorted(set(prior_groups)),
        "actor": (actor or "sweep"),
    }
    if days is not None:
        threshold = PERSON_QUARANTINE if classify(peer) == "person" else GEAR_QUARANTINE
        rec["reason"] = "inactive %.1fd (threshold %dd)" % (days, threshold)
    line = json.dumps(rec)
    with _hold_records_lock():
        # Under the sidecar lock, drop any existing line for this peer_id
        # and rewrite the file with a single current record. The validator
        # relies on one line per peer_id; a re-entry replaces the previous
        # line rather than appending a duplicate. Bad lines are skipped,
        # counted on stderr and carried through unchanged, never dropped.
        rows, malformed = _read_hold_lines()
        _warn_bad_hold_lines(malformed)
        survivors = [raw for raw, data in rows if data.get("peer_id") != rec["peer_id"]]
        _rewrite_hold_file(malformed + survivors + [line + "\n"])


def mark_hold_record_releasing(peer_id):
    """Rewrite the line for *peer_id* with `phase: releasing` under the
    lock, BEFORE the release touches any group. Returns the updated record
    or None when no line exists."""
    if not HOLD_RECORDS:
        return None
    with _hold_records_lock():
        rows, malformed = _read_hold_lines()
        _warn_bad_hold_lines(malformed)
        rec, out = None, []
        for raw, data in rows:
            if rec is None and data.get("peer_id") == peer_id:
                data = dict(data)
                data["phase"] = "releasing"
                rec = data
                out.append(json.dumps(data) + "\n")
                continue
            out.append(raw)
        if rec is not None:
            _rewrite_hold_file(malformed + out)
        return rec


def delete_hold_record(peer_id):
    """Remove the line for *peer_id* under the sidecar lock; return it or None."""
    if not HOLD_RECORDS:
        return None
    with _hold_records_lock():
        rows, malformed = _read_hold_lines()
        _warn_bad_hold_lines(malformed)
        rec = None
        survivors = []
        for raw, data in rows:
            if rec is None and data.get("peer_id") == peer_id:
                rec = data
                continue
            survivors.append(raw)
        if rec is not None:
            # A crash before os.replace leaves the original intact; a crash
            # after replaces it with the fully written survivors file.
            _rewrite_hold_file(malformed + survivors)
        return rec


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--apply", action="store_true")
    ap.add_argument("--lane", choices=["purge", "quarantine", "revoke", "release"])
    ap.add_argument("--grace-days", type=int, default=PURGE_GRACE_DEFAULT)
    ap.add_argument("--json", action="store_true")
    # Prove the mutation is correct on one record before doing it to hundreds.
    # The purge lane is the irreversible one; "run it and read the output" is
    # not a test when the first call may already be wrong.
    ap.add_argument("--actor", help="Operator handle; required with --apply.")
    ap.add_argument("--peer", help="Peer name or id for --lane release.")
    ap.add_argument(
        "--hold-entered-at",
        help="For --lane release, require the matched hold record to have this exact entered_at value.",
    )
    ap.add_argument(
        "--acknowledge-quarantined-is-dormancy", action="store_true",
        help="Required when --lane release finds NO hold record for the peer and "
             "would fall back to the ledger for a peer in <SITE>-QUARANTINED. "
             "QUARANTINED is also the security containment group (ADR 1091); a "
             "stale dormancy ledger row cannot tell the two apart. Passing this "
             "flag attests that this peer's hold is a pre-change dormancy hold "
             "and not a containment. Refused without it.")
    ap.add_argument("--limit", type=int)
    ap.add_argument("--entries-out",
                    help="Governed scope, --lane quarantine: write the hold entries here "
                         "instead of stdout.")
    a = ap.parse_args()
    if not SITE:
        print("GOATNET is not set: the goatnet this sweep reads "
              "(/etc/goat/lifecycle-sweep.env carries it, with NB_API, on a mgmt host).",
              file=sys.stderr)
        return 2
    governed = membership_file()
    if a.apply:
        if not a.lane:
            print("--apply requires --lane. There is no apply-everything: the lanes "
                  "have different reversibility and must be chosen deliberately.",
                  file=sys.stderr)
            return 2
        actor = (a.actor or "").strip()
        if not actor:
            print("--apply requires --actor <handle> on lane %s." % a.lane,
                  file=sys.stderr)
            return 2
        a.actor = actor
        if a.lane == "release" and not a.peer:
            print("--lane release requires --peer <name-or-id> when applying.",
                  file=sys.stderr)
            return 2
        if a.json:
            print("--json is the report-only output; it never applies. Drop --json to "
                  "apply, or drop --apply to report.", file=sys.stderr)
            return 2
        if governed and a.lane == "release":
            print("%s is governed by warrant (%s): a dormancy hold is an entry there, and "
                  "releasing it is deleting that entry in a pull request. Nothing is written "
                  "live." % (SITE, governed), file=sys.stderr)
            return 2
        if a.lane in ("quarantine", "release", "revoke") and not HOLD_RECORDS and not (
                governed and a.lane == "quarantine"):
            print("--apply lane %s requires LIFECYCLE_HOLD_RECORDS (set in /etc/goat/lifecycle-sweep.env)."
                  % a.lane,
                  file=sys.stderr)
            return 2

    if (a.peer or a.hold_entered_at or a.acknowledge_quarantined_is_dormancy) and a.lane != "release":
        print("--peer and --acknowledge-quarantined-is-dormancy only apply to --lane release; "
              "--hold-entered-at does too; "
              "lane %s acts on its whole candidate list (bound it with --limit)."
              % (a.lane or "(none)"), file=sys.stderr)
        return 2
    if a.limit is not None and a.limit < 1:
        print("--limit must be a positive number of peers.", file=sys.stderr)
        return 2
    if not a.apply and a.lane == "release":
        # Release has no report-only preview: it acts on one named peer's
        # record, so without --apply the flag changes nothing. Say so rather
        # than silently printing the ordinary roster.
        print("note: --lane release only acts with --apply --peer <name-or-id> "
              "--actor <handle>; printing the ordinary report instead.",
              file=sys.stderr)

    # NB_TOKEN first, and it is what the systemd unit supplies via its
    # EnvironmentFile. The two fallbacks below exist for hand-runs on a
    # goatnet's mgmt host; a scheduled guard must not depend on them. Scraping
    # the token out of a running container needs `sudo docker`, which forces the
    # unit to hold docker-socket write access and defeats the confinement the
    # guard-scheduling contract asks for. Both names carry the goatnet's own
    # prefix (<SITE>_NB_API_TOKEN is the name the onboard-runner role renders
    # and the runner reads; <SITE>_PORTAL_NB_SVC_TOKEN is the operator variable
    # ops/netbird/service-identities.yaml declares).
    site_env = SITE.upper().replace("-", "_")
    token = (os.environ.get("NB_TOKEN", "")
             or container_env("onboard-runner", site_env + "_NB_API_TOKEN")
             or os.environ.get(site_env + "_PORTAL_NB_SVC_TOKEN", "")).strip()
    if not token:
        print("no NetBird API token available", file=sys.stderr)
        return 2

    peers = api("/api/peers", token=token)
    keys = api("/api/setup-keys", token=token)
    if not peers:
        print("FAILED to read peers — NOT an all-clear.", file=sys.stderr)
        return 2
    groups = api("/api/groups", token=token)
    if not groups:
        print("FAILED to read groups — NOT an all-clear.", file=sys.stderr)
        return 2

    # ── dormancy lanes ──────────────────────────────────────────────────────
    lanes = {"quarantine": [], "revoke": []}
    refused = []
    exempt = []
    groups_by_name = {g["name"].upper(): g for g in groups}
    # Hold records by peer id, read once: a held device's class lives there.
    hold_by_id = {}
    if HOLD_RECORDS:
        for _, data in _read_hold_lines()[0]:
            hold_by_id[data.get("peer_id")] = data
    for p in peers:
        d = days_since(p.get("last_seen"))
        cls = classify(p)
        q, r = ((PERSON_QUARANTINE, PERSON_REVOKE) if cls == "person"
                else (GEAR_QUARANTINE, GEAR_REVOKE))
        row = {"name": p.get("name"), "id": p.get("id"), "class": cls,
               "days": round(d, 1) if d is not None else None}
        dormant = (not p.get("connected")) and d is not None
        exempt_class = explicit_revocation_only(p, hold_by_id.get(p.get("id")))
        if exempt_class and dormant and d >= q:
            hold_group, reason, group_names = choose_hold_group(p, groups_by_name)
            if is_inconsistent(reason):
                # A device in a hold group AND a class or role group is the
                # stranded-hold shape below; it stays visible there, not
                # filed away as exempt.
                refused.append({
                    "name": row["name"],
                    "id": row["id"],
                    "class": row["class"],
                    "days": row["days"],
                    "connected": bool(p.get("connected")),
                    "groups": group_names,
                    "reason": reason,
                })
                continue
            # ADR 1216: reported, never acted on by any lane.
            row["exempt_class"] = exempt_class
            row["would_be"] = "revoke" if d >= r else "quarantine"
            exempt.append(row)
            continue
        if dormant and d >= r:
            lanes["revoke"].append(row)
            continue
        hold_group, reason, group_names = choose_hold_group(p, groups_by_name)
        if is_inconsistent(reason):
            # A peer in a hold group AND a role group is inconsistent live
            # state whatever its connectivity: a stranded hold whose owner
            # came back online is dual-homed with full role access, and it
            # must stay visible (and resumable) rather than vanish from the
            # roster the moment the peer connects.
            refused.append({
                "name": row["name"],
                "id": row["id"],
                "class": row["class"],
                "days": row["days"],
                "connected": bool(p.get("connected")),
                "groups": group_names,
                "reason": reason,
            })
            continue
        if not (dormant and d >= q):
            continue
        if hold_group:
            row["hold_group"] = hold_group
            lanes["quarantine"].append(row)
        elif reason:
            refused.append({
                "name": row["name"],
                "id": row["id"],
                "class": row["class"],
                "days": row["days"],
                "connected": False,
                "groups": group_names,
                "reason": reason,
            })

    # ── never-enrolled ──────────────────────────────────────────────────────
    enrolled = set()
    for p in peers:
        m = NAMED.match(p.get("name") or "")
        if m:
            enrolled.add(m.group(1))
    # EVER enrolled, not enrolled NOW — ADR 1136 D2a. A handle whose peer was
    # revoked has history and belongs in the dormancy lanes, not the purge lane.
    # Without this, revoking a peer immediately makes its owner's key
    # purge-eligible and the next sweep deletes the credential of somebody who
    # was merely dormant. Observed the moment the revoke lane first ran: purge
    # went 0 -> 1 and the new row was the peer just revoked.
    try:
        with open(LEDGER) as fh:
            for line in fh:
                f = line.rstrip("\n").split("\t")
                if len(f) >= 3 and f[1] in ("revoke", "quarantine"):
                    m = NAMED.match(f[2])
                    if m:
                        enrolled.add(m.group(1))
    except FileNotFoundError:
        pass  # no ledger file yet
    except OSError as exc:
        print("FAILED to read the ledger %s (%s) — NOT an all-clear." % (LEDGER, exc), file=sys.stderr)
        return 2
    purge = []
    unageable = []
    for k in keys:
        name = k.get("name") or ""
        m = NAMED.match(name)
        if not m:
            # A `peer-` key that does not match the DATED grammar is not "not a
            # person's key" — it is a person's key nothing can age. `NAMED`
            # requires a YYYY-MM-DD, so a hand-made name like
            # `peer-<handle>-<goatnet>-clean` fell out here and never reached any
            # lane at all: it is INVISIBLE rather than mis-aged, which is why it
            # can sit through a full retirement pass unnoticed while being months
            # old and close to expiry.
            #
            # Surface it instead of dropping it. Never purge it — an unageable
            # key is a decision for an operator, and the destructive lane must
            # fail closed — but never hide it either, because a silent exemption
            # is indistinguishable from having no such key.
            if name.startswith("peer-") and (k.get("used_times") or 0) == 0:
                unageable.append({"handle": None, "key_id": k.get("id"), "key": name,
                                  "valid": bool(k.get("valid")),
                                  "age_clock": "unageable-no-date-in-name",
                                  "expires": k.get("expires")})
            continue
        h = m.group(1)
        if h in enrolled:
            continue                      # has a peer -> dormancy lanes, not here
        if (k.get("used_times") or 0) > 0:
            continue                      # was redeemed once; history exists
        age, clock = key_age_days(k)
        if age is None:
            # UNAGEABLE, and therefore never silently purged. Reported in its own
            # bucket so a key outside the grammar is a visible decision for an
            # operator rather than an invisible exemption — the previous
            # behaviour let one sit unnoticed through a full retirement pass.
            unageable.append({"handle": h, "key_id": k.get("id"), "key": k.get("name"),
                              "valid": bool(k.get("valid")), "age_clock": clock,
                              "expires": k.get("expires")})
            continue
        if age < a.grace_days:
            continue                      # inside the grace period
        purge.append({"handle": h, "key_id": k.get("id"), "key": k.get("name"),
                      "valid": bool(k.get("valid")), "age_days": round(age, 1),
                      "age_clock": clock})

    out = {"generated": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
           "thresholds": {"person": [PERSON_QUARANTINE, PERSON_REVOKE],
                          "gear": [GEAR_QUARANTINE, GEAR_REVOKE],
                          "purge_grace": a.grace_days},
           "quarantine": lanes["quarantine"], "revoke": lanes["revoke"],
           "purge": purge, "unageable": unageable, "refused": refused,
           "exempt": exempt,
           "hold_records": hold_records_summary(
               {p.get("id"): set(peer_group_names(p)) for p in peers})}

    if a.json:
        print(json.dumps(out, indent=1))
        return 0

    def bycls(rows, c):
        return len([r for r in rows if r["class"] == c])

    print("lifecycle sweep — %s   (ADR 1136)" % out["generated"])
    print("  peers known %d, of which connected %d"
          % (len(peers), len([p for p in peers if p.get("connected")])))
    print()
    print("  QUARANTINE   %3d   people %d (>=%dd) / gear %d (>=%dd)"
          % (len(lanes["quarantine"]), bycls(lanes["quarantine"], "person"),
             PERSON_QUARANTINE, bycls(lanes["quarantine"], "gear"), GEAR_QUARANTINE))
    print("  REVOKE       %3d   people %d (>=%dd) / gear %d (>=%dd)"
          % (len(lanes["revoke"]), bycls(lanes["revoke"], "person"),
             PERSON_REVOKE, bycls(lanes["revoke"], "gear"), GEAR_REVOKE))
    print("  PURGE        %3d   key issued, no peer ever, older than %dd"
          % (len(purge), a.grace_days))
    print("               %3d   of those still carry a VALID key"
          % len([p for p in purge if p["valid"]]))
    if exempt:
        print("  EXEMPT       %3d   dormant NETWORK / CONTROL / EDGE-SERVER peers — "
              "explicit revocation only, no lane acts on them (ADR 1216)" % len(exempt))
    if refused:
        inconsistent = [r for r in refused if is_inconsistent(r["reason"])]
        print("  REFUSED      %3d   hold-lane candidates refused by the group-choice rule"
              % len(refused))
        if inconsistent:
            print("               %3d   of those are INCONSISTENT (hold group + role group), "
                  "%d currently connected"
                  % (len(inconsistent),
                     len([r for r in inconsistent if r.get("connected")])))
    hr = out["hold_records"]
    if hr and hr.get("error"):
        print("  HOLD RECORDS   ?   unreadable at %s (%s)" % (hr["path"], hr["error"]))
    elif hr:
        print("  HOLD RECORDS %3d   on file at %s" % (hr["records"], hr["path"]))
        if hr["bad_lines_skipped"]:
            print("               %3d   bad line(s) skipped — those peers have no record "
                  "(reads as a security hold); run ops/lifecycle/validate-hold-records.py"
                  % hr["bad_lines_skipped"])
        if hr["orphans"]:
            print("               %3d   record(s) whose peer no longer exists — "
                  "--apply --lane release --peer <id> removes each" % len(hr["orphans"]))
            for o in hr["orphans"][:10]:
                print("                     %s (%s, in %s)" % (o["peer_id"], o["peer_name"], o["hold_group"]))
        if hr["stale"]:
            print("               %3d   record(s) whose peer is NOT in its hold group — "
                  "--apply --lane release --peer <id> removes each" % len(hr["stale"]))
            for o in hr["stale"][:10]:
                print("                     %s (%s, record says %s)" % (o["peer_id"], o["peer_name"], o["hold_group"]))
        if hr["releasing"]:
            print("               %3d   release(s) in progress — re-run --apply --lane release "
                  "--peer <id> to finish" % hr["releasing"])
    if unageable:
        live_un = len([u for u in unageable if u["valid"]])
        print("  UNAGEABLE    %3d   key name carries no date, so no lane can age it"
              % len(unageable))
        print("               %3d   of those still carry a VALID key — decide by hand"
              % live_un)
        for u in unageable[:10]:
            print("                     %s (expires %s)"
                  % (u["key"], (u.get("expires") or "?")[:10]))
        if len(unageable) > 10:
            print("                     ... and %d more" % (len(unageable) - 10))

    if not a.apply:
        for lane in ("quarantine", "revoke"):
            if lanes[lane]:
                print("\n  %s:" % lane)
                for r in sorted(lanes[lane], key=lambda x: -x["days"])[:12]:
                    print("    %-52s %-6s %6.1fd" % (r["name"][:52], r["class"], r["days"]))
                if len(lanes[lane]) > 12:
                    print("    ... and %d more" % (len(lanes[lane]) - 12))
        if exempt:
            print("\n  exempt (explicit revocation only):")
            for r in sorted(exempt, key=lambda x: -x["days"])[:12]:
                print("    %-52s %-11s %6.1fd  would be %s"
                      % (r["name"][:52], r["exempt_class"], r["days"], r["would_be"]))
            if len(exempt) > 12:
                print("    ... and %d more" % (len(exempt) - 12))
        if refused:
            print("\n  refused:")
            for r in refused[:12]:
                print("    %-52s %-6s %s  groups=%s  reason=%s"
                      % (r["name"][:52], r["class"], _fmt_days(r["days"]),
                         ",".join(r["groups"]), r["reason"]))
            if len(refused) > 12:
                print("    ... and %d more" % (len(refused) - 12))
        if governed:
            print("\n  GOVERNED     %s: quarantine/release are entries in %s, never live writes."
                  % (SITE, governed))
        print("\nREPORT ONLY — nothing changed. --apply --lane <name> to act.")
        return 0

    print("\nAPPLY: lane=%s" % a.lane)
    n = 0
    fail = 0
    # Idempotence:
    #   * revoke derives "already done" from the ledger history, by peer ID
    #     (revoked_peer_ids);
    #   * quarantine relies on live group membership via choose_hold_group,
    #     so stale hold-record lines never block a re-hold.
    os.makedirs(os.path.dirname(LEDGER), exist_ok=True)
    if a.lane == "purge":
        # Only handles whose revoke ACTUALLY SUCCEEDED go on the archive list
        # below. Building that list from the candidate set instead would tell an
        # operator to shelve the record of somebody whose key is still LIVE —
        # the revoke loop swallows API errors on purpose (one failure must not
        # abandon the rest), so "we tried" and "it worked" are different sets and
        # only the second may drive a record change.
        revoked = []
        for r in (purge[:a.limit] if a.limit else purge):
            try:
                api("/api/setup-keys/" + r["key_id"], "PUT",
                    {"name": r["key"], "revoked": True, "auto_groups": []}, token=token)
                n += 1
                revoked.append(r)
                print("  revoked unredeemed key  %s" % r["key"])
            except Exception as e:
                print("  FAILED %s — %s" % (r["key"], e))
        # THE CONSOLE HALF IS A SEPARATE ACTION, AND SAYING SO IS THE POINT.
        # This lane revokes the credential on the mesh; the person's proposal
        # record lives in the console's onboard.db, which this script has no
        # handle on — it authenticates to NetBird, not to the console, and it
        # knows handles rather than proposal ids.
        #
        # the console side EXISTS as archive-and-restore
        # (POST /api/onboard/proposals/{id}/archive), replacing the record
        # DELETION ADR 1136 originally specified. Archiving keeps the sponsor and
        # both gate approvals, so bringing somebody back is one admin action
        # rather than a fresh three-stage onboarding — which is the property that
        # makes this lane runnable at all.
        #
        # Naming the handles rather than a count: an operator archiving these by
        # hand needs to know WHO, and a lane that reports only a number is one
        # nobody can finish.
        print("\n%d unredeemed key(s) revoked." % n)
        if n:
            print("  These records are still `onboarded` in the console with a dead key.")
            print("  Archive them (keeps both gate approvals; Restore re-mints and re-sends):")
            for r in revoked[:20]:
                print("    %s" % r["handle"])
            if len(revoked) > 20:
                print("    ... and %d more" % (len(revoked) - 20))
        # Counted against what this run ACTED on, never the full candidate list:
        # under --limit the two differ, and reporting the candidate total next to
        # a truncated run reads as though everything was handled.
        skipped = len(purge) - (a.limit if a.limit else len(purge))
        if skipped > 0:
            print("\n  NOT TOUCHED THIS RUN: %d further candidate(s), held back by "
                  "--limit %d. Re-run to continue." % (skipped, a.limit))
    elif a.lane == "quarantine" and governed:
        # mesh-change-control.md §4.5: the hold is an entry in a PR, not a
        # live write. No ledger row and no hold record either — the membership
        # file is the record, and deleting the entry is the release.
        groups_now = api("/api/groups", token=token)
        byname = {g["name"].upper(): g for g in groups_now}
        entries = []
        for r in (lanes["quarantine"][:a.limit] if a.limit else lanes["quarantine"]):
            hg = byname.get((r.get("hold_group") or "").upper())
            if hg is None:
                fail += 1
                print("  REFUSED %s — hold group %r does not exist on this goatnet"
                      % (r["name"], r.get("hold_group")))
                continue
            entries.append(hold_entry(r, hg["name"], a.actor))
        for r in refused:
            print("  REFUSED %s — %s; groups=%s; no entry written"
                  % (r["name"], r["reason"], ",".join(r["groups"])))
        # New holds go into holds.yaml; a peer that already has an entry is
        # replaced in whichever fragment holds it (one entry per peer).
        where = entry_fragments(governed)
        fresh = [e for e in entries if e["peer"] not in where]
        replacing = [e for e in entries if e["peer"] in where]
        text = render_entries(fresh)
        if a.entries_out:
            with open(a.entries_out, "w") as fh:
                fh.write(text)
            print("\n%d hold entr%s written to %s — add under `entries:` in %s/%s."
                  % (len(fresh), "y" if len(fresh) == 1 else "ies", a.entries_out, SITE, HOLDS_FRAGMENT))
        else:
            print("\n# %d hold entr%s for %s — add under `entries:` in ops/warrant/membership/%s/%s "
                  "in a pull request:" % (len(fresh), "y" if len(fresh) == 1 else "ies", governed,
                                           SITE, HOLDS_FRAGMENT))
            sys.stdout.write(text)
        for e in replacing:
            print("\n# REPLACES the existing entry for %s in ops/warrant/membership/%s/%s:"
                  % (e["peer"], SITE, where[e["peer"]]))
            sys.stdout.write(render_entries([e]))
        print("%s is governed by warrant: nothing was written live. Each entry is a narrowing "
              "(mesh-change-control.md §4.3), so the PR merges without waiting; the membership "
              "converger applies it." % SITE)
    elif a.lane == "quarantine":
        # Dormancy hold: into the PROBATION mirror or PROBATION-GEAR, out of
        # every other group. `All` is NetBird's implicit membership and is
        # never removed.
        groups = api("/api/groups", token=token)
        byname = {g["name"].upper(): g for g in groups}
        live_ids = {p.get("id") for p in peers}
        name_counts = {}
        for p in peers:
            name_counts[p.get("name")] = name_counts.get(p.get("name"), 0) + 1
        attempted = 0
        for r in (lanes["quarantine"][:a.limit] if a.limit else lanes["quarantine"]):
            hold_group = r.get("hold_group")
            if not hold_group:
                print("  REFUSED %s — no hold group chosen, see report-only run" % r["name"])
                fail += 1
                continue
            hg = byname.get(hold_group.upper())
            if hg is None:
                print("  REFUSED %s — hold group %r does not exist on this goatnet"
                      % (r["name"], hold_group))
                fail += 1
                continue
            try:
                peer = api("/api/peers/" + r["id"], token=token)
                # The candidate list came from an earlier read. Re-run the
                # rule on the fresh peer: if an admin added a group since
                # (a QUARANTINED containment, a second role), the answer
                # changes and the hold must not go ahead on stale facts —
                # least of all record that containment as restore content
                # and remove it.
                fresh_hold, fresh_reason, fresh_names = choose_hold_group(peer, byname)
                if fresh_hold != hold_group:
                    fail += 1
                    print("  REFUSED %s — groups changed since the report (now %s): %s"
                          % (r["name"], ",".join(fresh_names),
                             fresh_reason or "no longer a candidate for %s" % hold_group))
                    continue
                attempted += 1        # from here on this candidate may write
                peer_groups = peer.get("groups") or []
                prior_ids = []
                prior_names = []
                for g in peer_groups:
                    name = (g.get("name") if isinstance(g, dict) else str(g)) or ""
                    gid = g.get("id") if isinstance(g, dict) else g
                    if name in ("All", hg["name"]):
                        continue
                    prior_ids.append(gid)
                    prior_names.append(name)
                # Intent BEFORE action: ledger row and record first. If the
                # hold-group PUT lands but its response is lost, the peer
                # is in its hold group and its role groups WITH a record,
                # which the next run finishes as a stranded hold. Written
                # after the add, that same crash left a peer with no record
                # — "needs hands" on every later run. A record whose peer is
                # not (yet) in its hold group is already handled: a re-hold
                # replaces it, and release removes it as STALE RECORD.
                with open(LEDGER, "a") as fh:
                    fh.write("%s\tquarantine\t%s\t%s\t%s\n" % (
                        time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
                        r["name"], r["id"], ",".join(prior_names)))
                append_hold_record(peer, hg["name"], prior_names, a.actor, r["days"], lane="dormancy")
                # Add to the hold group (re-read, then PUT).
                _group_set_member(hg["id"], r["id"], True, token)
                # Verify membership by re-reading the hold group.
                check = api("/api/groups/" + hg["id"], token=token)
                check_peers = [p["id"] if isinstance(p, dict) else p
                               for p in (check.get("peers") or [])]
                if r["id"] not in check_peers:
                    fail += 1
                    print("  FAILED %s — hold group %s did not retain the peer (record "
                          "written; the next run re-holds or release removes it)"
                          % (r["name"], hg["name"]))
                    continue
                # Remove from prior groups after the hold is in place.
                _remove_peer_from_groups(r["id"], prior_ids, token)
                n += 1
                print("  quarantined  %-46s %5.1fd  (into %s, prior %d group(s))"
                      % (r["name"][:46], r["days"], hg["name"], len(prior_names)))
            except Exception as exc:
                fail += 1
                print("  FAILED %s — %s" % (r["name"], exc))
            time.sleep(0.3)
        # Two kinds of refusal, and only one is a fault of this tool:
        #
        #   * The group-choice rule gave no single answer (two roles, no
        #     role, a role with no mirror). The peer is left in its full
        #     groups on purpose — that is the fail-closed outcome the rule
        #     exists for, and it is the standing state of every
        #     two-role person until the multi-group restore (#1108). It is
        #     printed and it does NOT fail the run: an exit code that is
        #     red on every apply distinguishes nothing.
        #
        #   * The peer is in a hold group AND a role group — inconsistent
        #     live state. A stranded hold (in its record's hold group, and
        #     holding nothing the record's prior_groups do not list) is
        #     finished here, because nothing else will: release refuses the
        #     contradiction and the next hold skips a held peer. Anything
        #     else in this shape needs hands AND counts as a failure, so the
        #     run exits 1 and a stuck peer cannot pass as a clean apply.
        resumed = 0
        left = 0
        # ONE --limit budget across holds and resumes, counted in ATTEMPTS
        # (a failed hold spent its slot too): --limit 1 means one peer
        # touched, not one held plus one resumed. Group-choice refusals
        # touch nothing, so they never spend the budget and are always
        # printed and counted — otherwise a stranded peer behind them would
        # never be reached under --limit, and the summary would under-count.
        budget = (a.limit - attempted) if a.limit else None
        deferred = 0
        for r in refused:
            if not is_inconsistent(r["reason"]):
                left += 1
                print("  REFUSED %s — %s; groups=%s; left in its groups (group-choice "
                      "rule, not a fault)" % (r["name"], r["reason"], ",".join(r["groups"])))
                continue
            if budget is not None and budget <= 0:
                deferred += 1
                continue
            if budget is not None:
                budget -= 1
            try:
                peer = api("/api/peers/" + r["id"], token=token)
                record = _lookup_hold_record(r["id"], r["name"], live_ids,
                                             name_counts.get(r["name"], 1))
                leftover = _resumable_hold(peer, record)
                if leftover is None:
                    fail += 1
                    if record and record.get("phase") == "releasing":
                        why = "a RELEASE is in progress; re-run --apply --lane release --peer %s" % r["id"]
                    elif record and record.get("lane") != "dormancy":
                        why = "its record is a %s hold, not this tool's" % record.get("lane")
                    else:
                        why = "no record, or groups the record never listed"
                    print("  REFUSED %s — %s; groups=%s; INCONSISTENT, needs hands (%s)"
                          % (r["name"], r["reason"], ",".join(r["groups"]), why))
                    continue
                removed = _remove_peer_from_groups(r["id"], leftover, token)
                resumed += 1
                print("  resumed      %-46s finished a stranded hold: removed from %d prior group(s)%s"
                      % (r["name"][:46], removed,
                         " (peer is connected)" if r.get("connected") else ""))
            except Exception as exc:
                fail += 1
                print("  FAILED %s — %s" % (r["name"], exc))
        if deferred:
            print("  NOT TOUCHED THIS RUN: %d inconsistent peer(s), held back by --limit %d. "
                  "Re-run to continue." % (deferred, a.limit))
        print("\n%d peer(s) quarantined, %d stranded hold(s) finished, %d left in place "
              "by the group-choice rule, %d failed. Prior membership is in %s; hold "
              "records in %s."
              % (n, resumed, left, fail, LEDGER, HOLD_RECORDS or "(unset)"))
    elif a.lane == "release":
        # Release is per peer and reads the hold record first, falling back to
        # the newest quarantine ledger row when no record exists.
        groups = api("/api/groups", token=token)
        byname = {g["name"]: g for g in groups}

        target = a.peer
        peer = None
        peer_id = None
        by_id = [p for p in peers if p.get("id") == target]
        by_nm = [p for p in peers if p.get("name") == target]
        if by_id:
            peer = by_id[0]
            peer_id = peer.get("id")
        elif len(by_nm) > 1:
            print("REFUSED %r — %d live peers share this name (%s); name the peer by id."
                  % (target, len(by_nm), ", ".join(p.get("id") or "?" for p in by_nm)))
            return 1
        elif by_nm:
            peer = by_nm[0]
            peer_id = peer.get("id")
        if peer_id is None:
            try:
                probe = api("/api/peers/" + target, token=token)
            except Exception:
                probe = None
            if probe:
                peer = probe
                peer_id = probe.get("id") or target
            else:
                # No such peer on the mesh. A record left behind (peer deleted
                # in the NetBird UI, or an old enrolment's id) would otherwise
                # sit forever and render as a held member with no exit.
                live = {p.get("id") for p in peers}
                orphan = _lookup_hold_record(target, target, live,
                                             len([p for p in peers if p.get("name") == target]) or 1)
                if orphan and orphan.get("lane") == "security":
                    print("REFUSED %s — its record is a SECURITY hold in %s (ADR 1091) and its "
                          "peer is gone; the containment's own close-out removes that line, "
                          "not this tool." % (orphan.get("peer_id"), orphan.get("hold_group")))
                    return 1
                if a.hold_entered_at and (
                        not orphan or orphan.get("entered_at") != a.hold_entered_at):
                    actual = orphan.get("entered_at") if orphan else "(none)"
                    print("REFUSED %s — hold identity changed: requested entered_at %s, current %s. "
                          "Nothing changed." % (target, a.hold_entered_at, actual))
                    return 1
                if orphan and orphan.get("peer_id") not in live:
                    delete_hold_record(orphan.get("peer_id"))
                    print("ORPHAN RECORD %s (%s) — no such peer on the mesh; removed its "
                          "hold-record line (was in %s, prior %s). Nothing else to do."
                          % (orphan.get("peer_id"), orphan.get("peer_name"),
                             orphan.get("hold_group"), ",".join(orphan.get("prior_groups") or []) or "(none)"))
                    return 0
                print("no peer named or id %r found and no record for it; nothing to release." % target)
                return 0
        # Decide on the peer as it is NOW, not as the start-of-run list had
        # it: a containment added since must be seen before role groups are
        # restored (the hold re-reads the same way before it acts).
        try:
            peer = api("/api/peers/" + peer_id, token=token) or peer
        except Exception as exc:
            print("FAILED %s — could not re-read the peer (%s). Nothing changed. If it was "
                  "deleted, re-run after the next report: the orphan exit removes its record."
                  % (peer.get("name"), exc))
            return 1

        live_ids = {p.get("id") for p in peers}
        same_name = len([p for p in peers if p.get("name") == peer.get("name")])
        record = _lookup_hold_record(peer_id, peer.get("name"), live_ids, max(same_name, 1))
        current_groups = peer_group_names(peer)
        current_upper = [n.upper() for n in current_groups]

        if a.hold_entered_at and (
                not record or record.get("entered_at") != a.hold_entered_at):
            actual = record.get("entered_at") if record else "(none)"
            print("REFUSED %s — hold identity changed: requested entered_at %s, current %s. "
                  "Nothing changed." % (peer.get("name"), a.hold_entered_at, actual))
            return 1

        prior_names = []
        hold_group = None
        if record:
            hold_group = record.get("hold_group")
            prior_names = record.get("prior_groups") or []
            if record.get("lane") == "security":
                # ADR 1091: a security hold's exit is the close-out of an
                # investigation, never a one-click release, and never this
                # tool. No flag overrides this.
                print("REFUSED %s — its record is a SECURITY hold in %s (ADR 1091); this "
                      "tool releases dormancy holds only. Resolve through the "
                      "containment's own close-out." % (peer.get("name"), hold_group))
                return 1
            extra_holds = [n for n in hold_groups_of(current_upper)
                           if n != (hold_group or "").upper()]
            if extra_holds:
                # A containment (or a second hold) placed ON TOP of the
                # dormancy hold. Restoring role groups now would hand a
                # contained peer its full reach — NetBird ACLs are additive.
                print("REFUSED %s — also in hold group(s) %s beyond its record's %s; a "
                      "release would restore role groups under a live containment. "
                      "Resolve by hand." % (peer.get("name"), ",".join(extra_holds), hold_group))
                return 1
            releasing = record.get("phase") == "releasing"
            if hold_group not in current_groups:
                # The record says held, the mesh says not: the peer was
                # released out of band (NetBird UI, an admin's hand). Left
                # alone, the line would sit forever and the console would
                # render a contradiction for a peer that is simply back. An
                # operator asked this tool to release this peer, so the
                # stale line is removed — said out loud, never silently.
                if releasing:
                    # The release may have died before its first restore and
                    # an admin may have emptied the hold group by hand since:
                    # put the prior groups back (idempotent) BEFORE the only
                    # copy of the restore content is deleted.
                    try:
                        for gn in prior_names:
                            _group_set_member(byname[gn]["id"], peer_id, True, token)
                    except (KeyError, Exception) as exc:
                        print("  FAILED %s — could not restore %s (%s); record kept as "
                              "releasing, re-run to retry." % (peer.get("name"), gn, exc))
                        return 1
                deleted = delete_hold_record(record.get("peer_id") or peer_id)
                print("%s %s — not in its record's hold group %s (%s); %sremoved the "
                      "hold-record line. Nothing else to do."
                      % ("RELEASE FINISHED" if releasing else "STALE RECORD", peer.get("name"),
                         hold_group, "an earlier release got that far" if releasing
                         else "released out of band, or never held",
                         "prior groups %s confirmed, " % (",".join(prior_names) or "(none)")
                         if releasing else ""))
                if deleted is None:
                    print("  (the line was already gone)")
                return 0
        else:
            # Ledger fallback for pre-change dormancy holds in QUARANTINED.
            # Membership FIRST: a peer that is not in QUARANTINED has nothing
            # this fallback could release, and saying "may be a security
            # containment" about a peer sitting in its role group (the
            # ordinary second run after a successful release) would be
            # false. The attestation gate only applies to a peer that IS in
            # QUARANTINED.
            qname = SITE.upper() + "-QUARANTINED"
            if qname not in current_groups:
                print("no hold record for %s and it is not in %s — nothing to release."
                      % (peer.get("name"), qname))
                return 0
            # Rows are this peer's by ID. A row matched by NAME is accepted
            # only when no id row exists and no other live peer shares the
            # name — otherwise a name collision (`Mac`) restores another
            # peer's groups, and the attestation flag attests dormancy, not
            # whose row this is.
            id_rows, name_rows = [], []
            try:
                with open(LEDGER) as fh:
                    for line in fh:
                        f = line.rstrip("\n").split("\t")
                        if len(f) >= 5 and f[1] == "quarantine":
                            row = {"name": f[2], "id": f[3],
                                   "groups": [x for x in f[4].split(",") if x]}
                            if f[3] == peer_id:
                                id_rows.append(row)
                            elif f[2] == peer.get("name") and f[3] not in live_ids:
                                name_rows.append(row)
            except FileNotFoundError:
                print("no ledger at %s — nothing to release." % LEDGER)
                return 0
            rows = id_rows or (name_rows if same_name <= 1 else [])
            if not rows:
                print("no hold record or ledger row for %s; it is in %s and so reads as a "
                      "security hold (ADR 1091) — nothing this tool will release."
                      % (peer.get("name"), qname))
                return 0
            row = rows[-1]
            extra_holds = [n for n in hold_groups_of(current_upper) if n != qname]
            if extra_holds:
                print("REFUSED %s — in %s and also in hold group(s) %s; more than one hold "
                      "at once is inconsistent live state. Resolve by hand."
                      % (peer.get("name"), qname, ",".join(extra_holds)))
                return 1
            if not a.acknowledge_quarantined_is_dormancy:
                # No record means security hold (README, "No record ⇒ security
                # hold"). A dormancy ledger row from an earlier episode does
                # not change that: a peer contained by an admin holds only
                # QUARANTINED, so no contradiction fires, and the fallback
                # would restore old groups and undo the containment (ADR
                # 1091). The operator attests it is a dormancy hold, or the
                # tool does nothing.
                print("REFUSED %s — no hold record; the ledger has a dormancy row for peer "
                      "id %s (prior groups %s) but %s may be a security containment. "
                      "Re-run with --acknowledge-quarantined-is-dormancy to attest "
                      "this is a pre-change dormancy hold."
                      % (peer.get("name"), row["id"], ",".join(row["groups"]) or "(none)", qname))
                return 1
            hold_group = qname
            prior_names = row["groups"]

        # Lane and group must agree (membership-lifecycle-and-transfer.md
        # §"Hold record"): a peer in a hold group AND a canonical role group is
        # inconsistent live state. Refuse and name both; never resolve it
        # silently by restoring on top of it.
        _, contradiction, _ = choose_hold_group(peer, {g["name"].upper(): g for g in groups})
        if record and record.get("phase") == "releasing" and is_inconsistent(contradiction):
            extra = [n.upper() for n in current_groups
                     if n != "All" and n != hold_group and n not in prior_names]
            if extra:
                print("REFUSED %s — a release is in progress but the peer also holds %s, "
                      "which its record never listed. Resolve by hand."
                      % (peer.get("name"), ",".join(sorted(extra))))
                return 1
            print("  resuming a release that stopped half-way for %s" % peer.get("name"))
        elif is_inconsistent(contradiction):
            if record and _resumable_hold(peer, record) is not None:
                print("REFUSED %s — inconsistent: %s. This is a stranded hold; finish it "
                      "with --apply --lane quarantine --actor <handle>, then release."
                      % (peer.get("name"), contradiction))
            else:
                print("REFUSED %s — inconsistent: %s. Resolve by hand before release."
                      % (peer.get("name"), contradiction))
            return 1

        missing = [gn for gn in prior_names if gn not in byname]
        if missing:
            print("REFUSED %s — prior group(s) %s no longer exist; a release would lose the "
                  "only copy of that restore content. Recreate the group or edit the record."
                  % (peer.get("name"), ",".join(missing)))
            return 1
        try:
            if record and record.get("phase") != "releasing":
                # Intent BEFORE action: once any group changes, the live
                # shape (hold group + prior groups) is the same as a stranded
                # hold's. The phase marker is what tells the next run to
                # finish the release rather than "finish the hold".
                mark_hold_record_releasing(record.get("peer_id") or peer_id)
            # Restore prior groups (re-read each group, then PUT).
            for gn in prior_names:
                _group_set_member(byname[gn]["id"], peer_id, True, token)
            # Remove from the hold group.
            hg = byname.get(hold_group)
            if hg:
                _group_set_member(hg["id"], peer_id, False, token)
            # Delete the hold-record line when one exists.
            if record:
                # Delete by the RECORD's id: a name-matched record belongs to
                # a previous enrolment of this peer and carries a different id.
                deleted = delete_hold_record(record.get("peer_id") or peer_id)
                if deleted:
                    print("  removed hold-record line for %s from %s"
                          % (peer.get("name"), deleted.get("hold_group")))
            n += 1
            print("  released  %-46s -> %s" % (peer.get("name")[:46], ",".join(prior_names)))
        except Exception as exc:
            fail += 1
            print("  FAILED %s — %s" % (peer.get("name"), exc))
        print("\n%d peer(s) released, %d failed." % (n, fail))
    elif a.lane == "revoke":
        # ADR 1031 mesh-first, reversible: DELETE the peer, never --purge. The
        # identity, the registry record and the attribution all survive, so
        # coming back is a re-enrolment through the ordinary portal flow rather
        # than a re-creation from nothing.
        #
        # This lane is NOT on a timer and is not reached by --apply without
        # --lane revoke. It removes someone's mesh access; that stays a decision
        # a person takes, on a roster they have looked at.
        #
        # "Already done" is decided by peer ID, not by name. Names are not
        # unique and are reused: once a peer called `Mac` has been revoked, a
        # later `Mac` (the same person re-enrolled, or an unrelated device
        # with the same unattributed name) is a different peer with its own
        # dormancy clock. Keyed by name, the lane skipped every one of them
        # forever with "already done" (GH #1305). Every revoke row has carried
        # the peer ID in column 4 since the lane was written, so nothing is
        # lost by ignoring the name here. The purge lane's "ever enrolled"
        # read of this ledger stays name-based on purpose: it is about handles.
        done_ids = revoked_peer_ids()
        todo = []
        for r in lanes["revoke"]:
            if r["id"] in done_ids:
                print("  skip (already done) %s (%s)" % (r["name"], r["id"])); continue
            todo.append(r)
        # --limit counts revokes still to make; a skip above spends none of it.
        for r in (todo[:a.limit] if a.limit else todo):
            try:
                # Record BEFORE the delete. After it the peer is gone from the
                # API and there is nothing left to read the groups from.
                peer = api("/api/peers/" + r["id"], token=token)
                sec = _lookup_hold_record(r["id"], None)
                if (sec and sec.get("lane") == "security") or any(
                        n.upper().endswith("-QUARANTINED") for n in peer_group_names(peer)):
                    # ADR 1091: a containment is closed out by its own
                    # process; a dormancy tool must not delete the peer and
                    # erase the record that documents the containment.
                    print("  skip (SECURITY hold, ADR 1091) %s — not this tool's to revoke" % r["name"])
                    continue
                prior = ",".join(sorted(
                    (g.get("name") if isinstance(g, dict) else str(g))
                    for g in (peer.get("groups") or [])))
                with open(LEDGER, "a") as fh:
                    fh.write("%s\trevoke\t%s\t%s\t%s\n" % (
                        time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
                        r["name"], r["id"], prior))
                api("/api/peers/" + r["id"], "DELETE", token=token)
                # Remove any hold-record line for this peer under the same lock
                # contract as other writers.
                deleted = delete_hold_record(r["id"])
                if deleted:
                    print("  removed hold-record line for %s from %s"
                          % (r["name"], deleted.get("hold_group")))
                n += 1
                print("  revoked  %-46s %5.1fd  (was in %s)"
                      % (r["name"][:46], r["days"], prior))
            except Exception as exc:
                fail += 1
                print("  FAILED %s — %s" % (r["name"], exc))
            time.sleep(0.3)
        print("\n%d peer(s) revoked, %d failed. Identities survive — re-onboarding "
              "is the ordinary portal flow. Prior groups are in %s." % (n, fail, LEDGER))
    else:
        print("  unknown lane %r" % a.lane)
        return 3
    return 1 if fail else 0


if __name__ == "__main__":
    sys.exit(main())
