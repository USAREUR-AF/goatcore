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
"""
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


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    n = sub.add_parser("new")
    n.add_argument("--agreement", required=True)
    n.add_argument("--us", required=True)
    n.add_argument("--out", required=True)
    n.add_argument("--hours", type=float, default=24)
    n.add_argument("--policy")
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
