#!/usr/bin/env python3
"""A bilateral acceptance session, exchanged by file (phase 14 run from both sides).

Each operator runs acceptance.sh on their own core and edge only. Neither holds the
other's credentials, and no service sits between them. They agree one session
manifest, both import it, both run, and each hands the other their result bundle as
a file. Each keeps their own observations and the partner's reported ones apart.

    acceptance_session.py new      --agreement A --us U --out M [--hours H] [--policy P]
    acceptance_session.py digest   M
    acceptance_session.py validate --manifest M --agreement A --us U
    acceptance_session.py fields   --manifest M --us U
    acceptance_session.py import   --manifest M --us U --evidence DIR BUNDLE
    acceptance_session.py combine  --manifest M --us U --evidence DIR
    acceptance_session.py propose  --manifest M --us U --evidence DIR --partner-file F
                                   --approved-by ID=NAME --approved-by ID=NAME --out DIR
    acceptance_session.py freshness --partner-file F --agreement A [--evidence DIR]

`new` writes the manifest: the agreement's digest, the acceptance policy's digest,
both participants, a unique session id, the window it is valid in, one random
challenge per participant and the sample limits. Read its `digest` aloud to the
partner and compare, as with the agreement.

`validate` refuses a manifest for another agreement, one that does not name us, one
not yet valid or expired, and a malformed one. acceptance.sh runs it first.

`fields` prints `<session id> <our challenge> <their challenge> <their id>`.

`import` checks a partner's result bundle and keeps it under
DIR/peer/<them>/<run id>.json: the schema; the session id and manifest digest; the
agreement digest; that it was observed by the partner, not by us; that its run lies
inside the session window and the session has not expired; that this run id was not
imported before (a replay); and that every inbound delivery the partner reports
carries OUR challenge, the bytes only this session's samples from us hold. It
refuses anything else with exit 2 and the reason.

`combine` writes DIR/bilateral/<session id>.json and .md: every check of ours and of
the partner's, side by side, local and peer-reported kept in separate columns. A
check we could not observe because the partner's edge or core is where it happens
(`observed_by_peer`) is satisfied only by the partner's mirror row passing. Exit 0
when the whole required scope of both sides passed, 1 when anything failed, 3
otherwise. The result says, every time, that peer-reported rows are not
authenticated: there is no report signing yet, so this is for the two operators to
review, and nothing gates on it automatically.

`propose` is the only way `exchanging` is published, and it publishes nothing
itself. It writes, under --out, the partner file with an `acceptance:` block and the
diff against the deployment's own copy, for the operator to commit through the
deployment's configuration review. It refuses unless the bilateral result is PASS,
both sides' bundles show their probes cleaned up (`cleanup/probes` PASS), both
operators are named as approvers, the session has not expired, and the partner
file's `level` already grants `exchanging`. It never edits the file in place and
never touches `level`.

`freshness` reads a partner file's `acceptance:` block and says whether it is still
current: stale when the agreement changed or the evidence expired, degraded when a
later local run failed. It prints the status change a person should propose; it
writes nothing.
"""
import difflib
import re
import argparse
import datetime
import hashlib
import importlib.util
import json
import os
import re
import secrets
import sys

SCHEMA = "goat-federation-acceptance-session/v1"
BUNDLE_SCHEMA = "goat-federation-acceptance/v1"
DEFAULT_POLICY = "every-generated-check-required/v1"
FMT = "%Y-%m-%dT%H:%M:%SZ"
# The session id, participant ids, challenges and run ids become key segments,
# payload bytes, file names and command arguments, so only these shapes are accepted.
SESSION_ID = re.compile(r"ses-[0-9A-Za-z-]{1,64}")
CHALLENGE = re.compile(r"[0-9a-f]{16,64}")
RUN_ID = re.compile(r"acc-[0-9A-Za-z-]{1,80}")
SOVEREIGN = re.compile(r"[a-z0-9][a-z0-9-]{1,15}")


class Refused(ValueError):
    pass


def utcnow():
    return datetime.datetime.now(datetime.timezone.utc).replace(microsecond=0)


def ts(d):
    return d.strftime(FMT)


def parse_ts(s):
    return datetime.datetime.strptime(s, FMT).replace(tzinfo=datetime.timezone.utc)


def sha256_bytes(b):
    return "sha256:" + hashlib.sha256(b).hexdigest()


def sha256_file(path):
    with open(path, "rb") as f:
        return sha256_bytes(f.read())


def load(path, what):
    try:
        with open(path) as f:
            return json.load(f)
    except (OSError, ValueError) as e:
        raise Refused("%s %s cannot be read: %s" % (what, path, e))


def parties(agreement):
    return {p.get("sovereign_id"): p for p in agreement.get("parties", [])}


def new_manifest(agreement_path, us, hours=24, policy_path=None, now=None):
    ag = load(agreement_path, "the agreement")
    ps = parties(ag)
    if us not in ps or len(ps) != 2:
        raise Refused("the agreement is not a two-party agreement naming %s" % us)
    now = now or utcnow()
    hz = [f.get("envelope", {}).get("hz") for f in ag.get("flows", [])]
    hz = [h for h in hz if isinstance(h, (int, float)) and h > 0]
    return {
        "schema": SCHEMA,
        "session_id": "ses-%s-%s" % (now.strftime("%Y%m%dT%H%M%SZ"), secrets.token_hex(6)),
        "agreement_sha256": sha256_file(agreement_path),
        "policy": {"name": os.path.basename(policy_path) if policy_path else DEFAULT_POLICY,
                   "sha256": sha256_file(policy_path) if policy_path else sha256_bytes(DEFAULT_POLICY.encode())},
        "participants": [{"sovereign_id": sid, "trust_domain": p.get("trust_domain"), "peering_cn": p.get("peering_cn")}
                         for sid, p in sorted(ps.items())],
        "created_by": us,
        "created_at": ts(now),
        "not_before": ts(now),
        "expires_at": ts(now + datetime.timedelta(hours=hours)),
        "challenges": {sid: secrets.token_hex(16) for sid in sorted(ps)},
        # One sample per offered set and direction, plus the republisher-boundary
        # sample, at no more than the agreement's lowest rate.
        "limits": {"samples_per_set": 2, "hz": min(hz) if hz else None},
    }


DERIVED_SCHEMA = "goat-federation-acceptance-session-derived/v1"
# The first line of the input the derived session id is hashed from, so it can
# never be confused with any other digest of the agreement.
DERIVED_DOMAIN = "goat-acceptance-session/v1\n"


def canonical_bytes(v):
    """Sorted keys, no whitespace, UTF-8: the bytes a derived session's digest covers."""
    return json.dumps(v, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode("utf-8")


def derive_session(agreement_path, us, attempt, policy_path=None):
    """The session both sides work out from the agreement they hold and the attempt
    number, with no file passed between them (§6.2). Nothing in it is random or read
    from a clock, so the two sides get the same bytes. Each side's challenge is not in
    it: a challenge stays that side's own and random, and travels with its samples."""
    if isinstance(attempt, bool) or not isinstance(attempt, int) or attempt < 1:
        raise Refused("the attempt %r is not a whole number from 1" % (attempt,))
    ag = load(agreement_path, "the agreement")
    ps = parties(ag)
    if us not in ps or len(ps) != 2:
        raise Refused("the agreement is not a two-party agreement naming %s" % us)
    expires = ag.get("expires")
    try:
        end = datetime.datetime.strptime(str(expires), "%Y-%m-%d")
    except ValueError:
        raise Refused("the agreement's expires %r is not a date YYYY-MM-DD" % (expires,))
    agreement_sha = sha256_file(agreement_path)
    sid = hashlib.sha256(("%s%s\n%d\n" % (DERIVED_DOMAIN, agreement_sha, attempt)).encode()).hexdigest()[:32]
    hz = [f.get("envelope", {}).get("hz") for f in ag.get("flows", [])]
    hz = [h for h in hz if isinstance(h, (int, float)) and h > 0]
    body = {
        "schema": DERIVED_SCHEMA,
        "session_id": "ses-" + sid,
        "attempt": attempt,
        "agreement_sha256": agreement_sha,
        "policy": {"name": os.path.basename(policy_path) if policy_path else DEFAULT_POLICY,
                   "sha256": sha256_file(policy_path) if policy_path else sha256_bytes(DEFAULT_POLICY.encode())},
        "participants": [{"sovereign_id": s, "trust_domain": p.get("trust_domain"), "peering_cn": p.get("peering_cn")}
                         for s, p in sorted(ps.items())],
        # A derived session lasts as long as the agreement it is derived from; each
        # run's own challenge, not a window, is what makes a result fresh.
        "expires_at": end.strftime("%Y-%m-%dT23:59:59Z"),
        "limits": {"samples_per_set": 2, "hz": min(hz) if hz else None},
    }
    return dict(body, digest=sha256_bytes(canonical_bytes(body)))


def compare_sessions(ours, theirs):
    """Refuse two derived sessions that are not the same, naming each member that
    differs, and a session whose digest does not cover its own members."""
    problems = []
    for name, s in (("ours", ours), ("theirs", theirs)):
        body = {k: v for k, v in s.items() if k != "digest"}
        if s.get("digest") != sha256_bytes(canonical_bytes(body)):
            problems.append("%s's digest does not cover its own members" % name)
    differ = sorted(k for k in set(ours) | set(theirs) if k != "digest" and ours.get(k) != theirs.get(k))
    if differ:
        problems.append("the two sides derived different sessions: %s differ" % ", ".join(differ))
    if problems:
        raise Refused("%s; both sides must hold the same agreement and attempt (GB-274)" % "; ".join(problems))


def validate(manifest, agreement_path, us, now=None):
    if manifest.get("schema") != SCHEMA:
        raise Refused("not a session manifest (schema %r)" % manifest.get("schema"))
    for k in ("session_id", "agreement_sha256", "participants", "not_before", "expires_at", "challenges"):
        if not manifest.get(k):
            raise Refused("the manifest has no %s" % k)
    if not SESSION_ID.fullmatch(str(manifest["session_id"])):
        raise Refused("the session id %r is not of the form ses-<letters, digits, hyphens>" % manifest["session_id"])
    ids = [p.get("sovereign_id") for p in manifest["participants"]]
    if not all(SOVEREIGN.fullmatch(str(i)) for i in ids):
        raise Refused("a participant id is not a sovereign id: %s" % ids)
    if not all(CHALLENGE.fullmatch(str(c)) for c in manifest["challenges"].values()):
        raise Refused("a challenge is not 16 to 64 lowercase hex digits")
    if us not in ids or len(ids) != 2:
        raise Refused("the manifest's participants %s do not name %s and one partner" % (ids, us))
    if set(manifest["challenges"]) != set(ids) or not all(manifest["challenges"].values()):
        raise Refused("the manifest needs one challenge per participant")
    if sha256_file(agreement_path) != manifest["agreement_sha256"]:
        raise Refused("the manifest was made for another agreement (%s), not %s" % (
            manifest["agreement_sha256"], sha256_file(agreement_path)))
    now = now or utcnow()
    try:
        nb, exp = parse_ts(manifest["not_before"]), parse_ts(manifest["expires_at"])
    except ValueError as e:
        raise Refused("the manifest's window is malformed: %s" % e)
    if now < nb:
        raise Refused("the session is not valid before %s" % manifest["not_before"])
    if now >= exp:
        raise Refused("the session expired at %s" % manifest["expires_at"])
    return [i for i in ids if i != us][0]


def _shape(m, us):
    """The manifest's own shape, as validate checks it, without the agreement: a
    well-formed session id, exactly two well-formed participants of whom `us` is one,
    and one well-formed challenge for each. Returns (manifest, partner id)."""
    if m.get("schema") != SCHEMA or not SESSION_ID.fullmatch(str(m.get("session_id"))):
        raise Refused("not a session manifest of this schema with a well-formed session id")
    ids = [str(p.get("sovereign_id")) for p in (m.get("participants") or []) if isinstance(p, dict)]
    if len(ids) != 2 or len(set(ids)) != 2 or not all(SOVEREIGN.fullmatch(i) for i in ids):
        raise Refused("the manifest needs exactly two participants with well-formed sovereign ids, has %s" % ids)
    if us not in ids:
        raise Refused("the manifest's participants %s do not include %s" % (ids, us))
    ch = m.get("challenges") or {}
    if set(ch) != set(ids) or not all(CHALLENGE.fullmatch(str(c)) for c in ch.values()):
        raise Refused("the manifest needs one challenge of 16 to 64 lowercase hex digits per participant")
    return m, [i for i in ids if i != us][0]


def import_bundle(manifest_path, us, evidence, bundle_path, now=None):
    m, them = _shape(load(manifest_path, "the session manifest"), us)
    b = load(bundle_path, "the bundle")
    if b.get("schema") != BUNDLE_SCHEMA:
        raise Refused("not an acceptance bundle (schema %r)" % b.get("schema"))
    run = b.get("run") or {}
    if not RUN_ID.fullmatch(str(run.get("id"))):
        raise Refused("the bundle's run id %r is not of the form acc-<letters, digits, hyphens>" % run.get("id"))
    if run.get("observed_by") != them or run.get("us") != them:
        raise Refused("the bundle was observed by %r, not by the partner %s" % (run.get("observed_by"), them))
    sess = b.get("session") or {}
    if sess.get("id") != m.get("session_id"):
        raise Refused("the bundle belongs to session %r, not %s" % (sess.get("id"), m.get("session_id")))
    if sess.get("manifest_sha256") != sha256_file(manifest_path):
        raise Refused("the bundle was run against another manifest (%s)" % sess.get("manifest_sha256"))
    if (b.get("agreement") or {}).get("sha256") != m.get("agreement_sha256"):
        raise Refused("the bundle observed another agreement (%s)" % (b.get("agreement") or {}).get("sha256"))
    if sess.get("challenge") != (m.get("challenges") or {}).get(them):
        raise Refused("the bundle carries another challenge than the one this session issued to %s" % them)
    now = now or utcnow()
    try:
        nb, exp = parse_ts(m["not_before"]), parse_ts(m["expires_at"])
        start, end = parse_ts(run["started_at"]), parse_ts(run["ended_at"])
    except (KeyError, ValueError) as e:
        raise Refused("the bundle or the manifest has no readable time window: %s" % e)
    if now >= exp:
        raise Refused("the session expired at %s; its bundles are no longer accepted" % m["expires_at"])
    if start < nb or end > exp:
        raise Refused("the bundle's run (%s to %s) lies outside the session window" % (run["started_at"], run["ended_at"]))
    ours = (m.get("challenges") or {}).get(us)
    for c in b.get("checks", []):
        if c.get("name", "").startswith("delivery/inbound/") and c.get("verdict") == "PASS":
            if ours not in (c.get("observations") or {}).get("received", ""):
                raise Refused("%s reports a delivery that does not carry our challenge: it is not from this session"
                              % c["name"])
    d = os.path.join(evidence, "peer", them)
    os.makedirs(d, exist_ok=True)
    index_path = os.path.join(d, "imported.json")
    index = {}
    if os.path.exists(index_path):
        index = load(index_path, "the import index")
    rid = str(run.get("id"))
    digest = sha256_file(bundle_path)
    # A bundle is accepted once: by its digest, and by its run id.
    if digest in index or any(v.get("run") == rid for v in index.values()):
        raise Refused("this bundle (%s, run %s) was already imported: a bundle is accepted once" % (digest, rid))
    safe = rid
    stored = dict(b)
    # Kept beside our own evidence, never mixed into it.
    stored["imported"] = {"at": ts(now), "from_file": os.path.basename(bundle_path), "sha256": digest,
                          "peer_reported": True, "authenticated": False}
    with open(os.path.join(d, safe + ".json"), "w") as f:
        json.dump(stored, f, indent=2)
        f.write("\n")
    index[digest] = {"run": rid, "session": sess.get("id"), "imported_at": ts(now)}
    with open(index_path, "w") as f:
        json.dump(index, f, indent=2, sort_keys=True)
        f.write("\n")
    return them, os.path.join(d, safe + ".json")


def mirror(name):
    """The partner's name for the same observation: our outbound is their inbound."""
    for a, b in (("/outbound", "/inbound"), ("/inbound", "/outbound")):
        if a in name:
            return name.replace(a, b, 1)
    return None


def _evidence_module():
    spec = importlib.util.spec_from_file_location("acceptance_evidence",
                                                  os.path.join(os.path.dirname(os.path.abspath(__file__)),
                                                               "acceptance_evidence.py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


# Rows each side keeps for itself; the partner has no counterpart to them.
LOCAL_ONLY = ("publish/", "existing/", "cleanup/earlier-runs", "fresh-keys", "lanes")


def expected_peer_rows(local):
    """The rows the partner's bundle must hold: the mirror of each of our required
    rows (our outbound is their inbound; rows without a direction keep their name).
    Our rows are generated from the agreement, so these are too."""
    out = []
    for c in local.get("checks", []):
        if not c.get("required") or c["name"].startswith(LOCAL_ONLY):
            continue
        out.append(mirror(c["name"]) or c["name"])
    return out


def combine(manifest_path, us, evidence):
    m, them = _shape(load(manifest_path, "the session manifest"), us)
    sid = m.get("session_id")
    ev = _evidence_module()
    local = load(os.path.join(evidence, "result.json"), "our result")
    if (local.get("session") or {}).get("id") != sid:
        raise Refused("our last result belongs to session %r, not %s; run phase 14 with this manifest" % (
            (local.get("session") or {}).get("id"), sid))
    d = os.path.join(evidence, "peer", them)
    peer = None
    if os.path.isdir(d):
        found = []
        for fn in sorted(os.listdir(d)):
            if fn.endswith(".json") and fn != "imported.json":
                b = load(os.path.join(d, fn), "an imported bundle")
                if (b.get("session") or {}).get("id") == sid:
                    found.append(b)
        if found:
            peer = max(found, key=lambda b: b["run"]["ended_at"])
    local_checks = {c["name"]: c for c in local.get("checks", [])}
    peer_checks = {}
    malformed = {}
    for c in (peer or {}).get("checks", []):
        # Re-checked on our side, with the partner's own `required` ignored.
        try:
            ev.make_check(c.get("name"), c.get("direction"), c.get("verdict"), c.get("reason"), c.get("started_at"),
                          ended=c.get("ended_at"), required=c.get("verdict") != "NOT APPLICABLE")
            peer_checks[c["name"]] = c
        except (ev.Malformed, TypeError, KeyError) as e:
            malformed[str(c.get("name"))] = str(e)

    def resolve(c, by_other, other_checks, kind):
        """Status of a row: a BLOCKED row the other side observes is decided by theirs."""
        if c["verdict"] == "BLOCKED" and by_other:
            oc = other_checks.get(by_other)
            if oc and oc["verdict"] in ("PASS", "FAIL"):
                return "%s (%s)" % (oc["verdict"], kind)
            return "OPEN"
        return c["verdict"]

    rows, failed, open_ = [], [], []
    for c in local.get("checks", []):
        by_peer = (c.get("observations") or {}).get("observed_by_peer")
        mc = peer_checks.get(by_peer) if by_peer else None
        rows.append({"name": c["name"], "required": c["required"], "local": c["verdict"],
                     "peer_reported": {"name": by_peer, "verdict": mc["verdict"] if mc else None} if by_peer else None,
                     "status": resolve(c, by_peer, peer_checks, "peer-reported")})
    expected = expected_peer_rows(local)
    for name in expected:
        c = peer_checks.get(name)
        if c is None:
            st = "FAIL (peer-reported, malformed)" if name in malformed else "OPEN"
            rows.append({"name": "%s: %s" % (them, name), "required": True, "local": None,
                         "peer_reported": {"name": name, "verdict": None}, "status": st})
            continue
        by_us = (c.get("observations") or {}).get("observed_by_peer")
        st = resolve(c, by_us, local_checks, "local")
        if st in ("PASS", "FAIL"):
            st += " (peer-reported)"
        rows.append({"name": "%s: %s" % (them, name), "required": True, "local": None,
                     "peer_reported": {"name": name, "verdict": c["verdict"]}, "status": st})
    for name, c in peer_checks.items():
        if name in expected:
            continue
        st = c["verdict"] + " (peer-reported)" if c["verdict"] in ("PASS", "FAIL") else c["verdict"]
        rows.append({"name": "%s: %s" % (them, name), "required": False, "local": None,
                     "peer_reported": {"name": name, "verdict": c["verdict"]}, "status": st})
    for name, why in malformed.items():
        if name not in expected:
            rows.append({"name": "%s: %s" % (them, name), "required": False, "local": None,
                         "peer_reported": {"name": name, "verdict": None}, "status": "FAIL (peer-reported, malformed)"})
    for r in rows:
        if r["status"].startswith("FAIL"):
            failed.append(r["name"])
        elif r["required"] and not r["status"].startswith("PASS"):
            open_.append(r["name"])
    if peer is None:
        open_.append("no bundle from %s imported for this session" % them)
    verdict = "FAIL" if failed else ("INCOMPLETE" if open_ else "PASS")
    out = {
        "schema": "goat-federation-acceptance-bilateral/v1",
        "session": sid, "manifest_sha256": sha256_file(manifest_path), "us": us, "them": them,
        "local_run": local["run"]["id"], "peer_run": (peer or {}).get("run", {}).get("id"),
        "verdict": verdict, "failed": failed, "open": open_,
        "authenticated": False,
        "gate": "operator review required: peer-reported rows are the partner's own report and are not "
                "authenticated; no automatic decision may rest on them",
        "rows": rows,
    }
    bd = os.path.join(evidence, "bilateral")
    os.makedirs(bd, exist_ok=True)
    with open(os.path.join(bd, sid + ".json"), "w") as f:
        json.dump(out, f, indent=2)
        f.write("\n")
    lines = ["# Bilateral acceptance: %s <-> %s, session %s" % (us, them, sid), "",
             "Verdict: **%s**. %s." % (verdict, out["gate"]), "",
             "| Check | Required | Ours (local) | Theirs (peer-reported) | Status |", "|---|---|---|---|---|"]
    for r in rows:
        pr = r["peer_reported"]
        lines.append("| `%s` | %s | %s | %s | %s |" % (
            r["name"], "yes" if r["required"] else "no", r["local"] or "", ("%s %s" % (pr["name"], pr["verdict"])) if pr else "",
            r["status"]))
    with open(os.path.join(bd, sid + ".md"), "w") as f:
        f.write("\n".join(lines) + "\n")
    return out


def _bundle_path(evidence, them, run_id):
    safe = "".join(ch if ch.isalnum() or ch in "-_." else "_" for ch in str(run_id))
    return os.path.join(evidence, "peer", them, safe + ".json")


def _cleanup_confirmed(bundle):
    return any(c.get("name") == "cleanup/probes" and c.get("verdict") == "PASS" for c in bundle.get("checks", []))


ACCEPTANCE_BLOCK = re.compile(r"^acceptance:[^\n]*\n(?:(?:[ \t]+[^\n]*|[ \t]*)\n)*", re.M)


def propose(manifest_path, us, evidence, partner_file, approvals, out_dir, now=None):
    target = os.path.join(out_dir, os.path.basename(partner_file))
    if os.path.realpath(os.path.dirname(target)) == os.path.realpath(os.path.dirname(os.path.abspath(partner_file))):
        raise Refused("--out is the partner file's own directory; the proposal is written beside it, never over it")
    m, them = _shape(load(manifest_path, "the session manifest"), us)
    now = now or utcnow()
    if now >= parse_ts(m["expires_at"]):
        raise Refused("the session expired at %s; run a new one before publishing" % m["expires_at"])
    out = combine(manifest_path, us, evidence)
    if out["verdict"] != "PASS":
        raise Refused("the bilateral result is %s (failed: %s; open: %s); exchanging is published only after the "
                      "whole required scope of both sides passed" % (
                          out["verdict"], ", ".join(out["failed"]) or "none", ", ".join(out["open"]) or "none"))
    local = load(os.path.join(evidence, "result.json"), "our result")
    peer_path = _bundle_path(evidence, them, out["peer_run"])
    peer = load(peer_path, "the partner's imported bundle")
    for side, b in ((us, local), (them, peer)):
        if not _cleanup_confirmed(b):
            raise Refused("cleanup is not confirmed on %s's side: its bundle has no cleanup/probes PASS "
                          "(run with a probe launcher)" % side)
    names = dict(a.split("=", 1) for a in approvals if "=" in a)
    for side in (us, them):
        if not names.get(side, "").strip():
            raise Refused("both operators approve the result: no approver named for %s (--approved-by %s=<name>)"
                          % (side, side))
        if any(ord(ch) < 32 or ord(ch) == 127 for ch in names[side]) or len(names[side]) > 120:
            raise Refused("the approver named for %s holds a control character or is longer than 120 characters"
                          % side)
    try:
        with open(partner_file) as f:
            text = f.read()
    except OSError as e:
        raise Refused("the partner file %s cannot be read: %s" % (partner_file, e))
    lv = re.search(r"^level:\s*([A-Za-z]+)", text, re.M)
    if not lv or lv.group(1) != "exchanging":
        raise Refused("the partner file's level is %r; acceptance records evidence and never widens the grant, so it "
                      "cannot publish exchanging over a level that does not move data" % (lv.group(1) if lv else None))
    sid = re.search(r"^sovereign_id:\s*([A-Za-z0-9-]+)", text, re.M)
    if not sid or sid.group(1) != them:
        raise Refused("the partner file declares %r, not this session's partner %s" % (sid.group(1) if sid else None,
                                                                                         them))
    ag_expires = None
    try:
        with open(local["agreement"]["path"]) as f:
            ag_expires = json.load(f).get("expires")
    except (OSError, ValueError, KeyError, TypeError):
        pass
    combined_path = os.path.join(evidence, "bilateral", m["session_id"] + ".json")
    block = "\n".join([
        "acceptance:",
        "  # Published through review of this change (docs/design/federation-acceptance.md section 7).",
        "  # It records evidence; it does not change level, and it goes stale on its own.",
        "  status: exchanging",
        '  session: "%s"' % m["session_id"],
        '  manifest_sha256: "%s"' % sha256_file(manifest_path),
        '  agreement_sha256: "%s"' % m["agreement_sha256"],
        '  local_bundle_sha256: "%s"' % sha256_file(os.path.join(evidence, "result.json")),
        '  peer_bundle_sha256: "%s"' % (peer.get("imported") or {}).get("sha256", sha256_file(peer_path)),
        '  combined_sha256: "%s"' % sha256_file(combined_path),
        '  verified_at: "%s"' % ts(now),
        '  expires: "%s"' % (ag_expires or m["expires_at"][:10]),
        "  approved_by:",
        # JSON strings are valid YAML double-quoted scalars, so a name is written
        # exactly, whatever quotes or backslashes it holds.
        "    %s: %s" % (us, json.dumps(names[us])),
        "    %s: %s" % (them, json.dumps(names[them])),
    ]) + "\n"
    if not text.endswith("\n"):
        text += "\n"
    if ACCEPTANCE_BLOCK.search(text):
        new = ACCEPTANCE_BLOCK.sub(lambda _: block, text, count=1)
    else:
        new = text + ("" if text.endswith("\n") else "\n") + "\n" + block
    os.makedirs(out_dir, exist_ok=True)
    name = os.path.basename(partner_file)
    proposed = os.path.join(out_dir, name)
    with open(proposed, "w") as f:
        f.write(new)
    diff = "".join(difflib.unified_diff(text.splitlines(True), new.splitlines(True), "a/" + name, "b/" + name))
    with open(os.path.join(out_dir, name + ".diff"), "w") as f:
        f.write(diff)
    return proposed, diff


def freshness(partner_file, agreement_path, evidence=None, now=None):
    """(status, reason) of a partner file's acceptance record: current, stale, degraded
    or none."""
    try:
        import yaml
    except ImportError:
        raise Refused("PyYAML is needed to read the partner file")
    try:
        with open(partner_file) as f:
            d = yaml.safe_load(f) or {}
    except (OSError, yaml.YAMLError) as e:
        raise Refused("the partner file %s cannot be read: %s" % (partner_file, e))
    ac = d.get("acceptance")
    if not isinstance(ac, dict) or ac.get("status") != "exchanging":
        return "none", "no published acceptance in %s" % partner_file
    now = now or utcnow()
    if ac.get("agreement_sha256") != sha256_file(agreement_path):
        return "stale", "the agreement changed since the acceptance was published (%s, now %s)" % (
            ac.get("agreement_sha256"), sha256_file(agreement_path))
    try:
        if datetime.date.fromisoformat(str(ac.get("expires"))[:10]) < now.date():
            return "stale", "the acceptance evidence expired on %s" % ac.get("expires")
    except ValueError:
        return "stale", "the acceptance record has no readable expiry"
    if evidence and os.path.exists(os.path.join(evidence, "result.json")):
        r = load(os.path.join(evidence, "result.json"), "our result")
        try:
            later = parse_ts(r["run"]["ended_at"]) > parse_ts(str(ac.get("verified_at")))
        except (KeyError, ValueError):
            later = False
        if later and r.get("verdict") == "FAIL":
            return "degraded", "a later local run (%s) failed: %s" % (r["run"]["id"], r.get("why", ""))
    return "current", "published %s for session %s; the agreement is unchanged and the evidence has not expired" % (
        ac.get("verified_at"), ac.get("session"))


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    n = sub.add_parser("new")
    n.add_argument("--agreement", required=True)
    n.add_argument("--us", required=True)
    n.add_argument("--out", required=True)
    n.add_argument("--hours", type=float, default=24)
    n.add_argument("--policy")
    dv = sub.add_parser("derive")
    dv.add_argument("--agreement", required=True)
    dv.add_argument("--us", required=True)
    dv.add_argument("--attempt", type=int, required=True)
    dv.add_argument("--policy")
    dv.add_argument("--out", required=True)
    g = sub.add_parser("digest")
    g.add_argument("manifest")
    v = sub.add_parser("validate")
    v.add_argument("--manifest", required=True)
    v.add_argument("--agreement", required=True)
    v.add_argument("--us", required=True)
    fl = sub.add_parser("fields")
    fl.add_argument("--manifest", required=True)
    fl.add_argument("--us", required=True)
    im = sub.add_parser("import")
    im.add_argument("--manifest", required=True)
    im.add_argument("--us", required=True)
    im.add_argument("--evidence", required=True)
    im.add_argument("bundle")
    co = sub.add_parser("combine")
    co.add_argument("--manifest", required=True)
    co.add_argument("--us", required=True)
    co.add_argument("--evidence", required=True)
    pr = sub.add_parser("propose")
    pr.add_argument("--manifest", required=True)
    pr.add_argument("--us", required=True)
    pr.add_argument("--evidence", required=True)
    pr.add_argument("--partner-file", required=True)
    pr.add_argument("--approved-by", action="append", default=[])
    pr.add_argument("--out", required=True)
    fr = sub.add_parser("freshness")
    fr.add_argument("--partner-file", required=True)
    fr.add_argument("--agreement", required=True)
    fr.add_argument("--evidence")
    a = ap.parse_args(argv)
    try:
        if a.cmd == "new":
            m = new_manifest(a.agreement, a.us, a.hours, a.policy)
            if os.path.exists(a.out):
                raise Refused("%s exists; a session manifest is written once" % a.out)
            with open(a.out, "w") as f:
                json.dump(m, f, indent=2)
                f.write("\n")
            print("session %s, valid until %s" % (m["session_id"], m["expires_at"]))
            print("manifest digest %s: read it aloud to the partner and compare" % sha256_file(a.out))
            return 0
        if a.cmd == "derive":
            d = derive_session(a.agreement, a.us, a.attempt, a.policy)
            with open(a.out, "w") as f:
                json.dump(d, f, indent=2, sort_keys=True)
                f.write("\n")
            print("session %s, attempt %d, valid until %s" % (d["session_id"], d["attempt"], d["expires_at"]))
            print("session digest %s: the partner derives the same from the same agreement" % d["digest"])
            return 0
        if a.cmd == "digest":
            print(sha256_file(a.manifest))
            return 0
        if a.cmd == "validate":
            them = validate(load(a.manifest, "the session manifest"), a.agreement, a.us)
            print("session manifest valid for %s with %s" % (a.us, them))
            return 0
        if a.cmd == "fields":
            m, them = _shape(load(a.manifest, "the session manifest"), a.us)
            print(m["session_id"], m["challenges"][a.us], m["challenges"][them], them)
            return 0
        if a.cmd == "import":
            them, path = import_bundle(a.manifest, a.us, a.evidence, a.bundle)
            print("imported %s's bundle as peer-reported (not authenticated): %s" % (them, path))
            return 0
        if a.cmd == "propose":
            proposed, diff = propose(a.manifest, a.us, a.evidence, a.partner_file, a.approved_by, a.out)
            print(diff, end="")
            print("proposed: %s (the deployment's file is unchanged; commit this through its review)" % proposed)
            return 0
        if a.cmd == "freshness":
            status, reason = freshness(a.partner_file, a.agreement, a.evidence)
            print("%s: %s" % (status, reason))
            if status in ("stale", "degraded"):
                print("propose: acceptance.status: %s in %s, through review; level is not changed" % (
                    status, a.partner_file))
            return 0 if status == "current" else 3
        out = combine(a.manifest, a.us, a.evidence)
        print("bilateral verdict %s (%d failed, %d open). %s." % (out["verdict"], len(out["failed"]), len(out["open"]),
                                                                  out["gate"]))
        return {"PASS": 0, "FAIL": 1}.get(out["verdict"], 3)
    except Refused as e:
        print("acceptance session: %s" % e, file=sys.stderr)
        return 2
    except OSError as e:
        print("acceptance session: %s" % e, file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
