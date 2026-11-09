#!/usr/bin/env python3
"""The evidence bundle of a federation acceptance run (phase 14), and its verdict.

acceptance.sh records every check here as it decides it, then asks for the verdict.
The verdict and the exit status come from this file only, so the rule that a check
which did not run is not a pass lives in one place, with tests.

    acceptance_evidence.py record   <checks.jsonl> --name N --direction D --verdict V
                                    --reason R --started T [--title S] [--tier T]
                                    [--optional] [--obs-file F]
    acceptance_evidence.py finalize <checks.jsonl> --out DIR --agreement A --us U
                                    --them T --run-id ID --started T [--inputs k=0|1 ...]
    acceptance_evidence.py rest-value  < curl output ending in a line with the HTTP status
    acceptance_evidence.py persistence-scope --key K --router F --subscribers F

`record` appends one check. `finalize` writes DIR/result.json (schema
goat-federation-acceptance/v1), DIR/summary.md and DIR/history/<run-id>.json, prints
the counts line and the verdict, and exits with the run's status:

    0  PASS        every required check passed
    1  FAIL        a check failed (an observed violation)
    3  INCOMPLETE  nothing failed, but a required check was BLOCKED or SKIPPED, or no
                   required check ran at all: the peering is not proven
    2  the bundle could not be written, or a record was malformed

Outcome words (the acceptance contract):
    PASS            the expected evidence was observed
    FAIL            a violation was observed
    BLOCKED         a prerequisite or the evidence was unavailable
    SKIP            not exercised
    NOT APPLICABLE  excluded by the agreed scope (never required; needs a reason)

A required check can only become PASS by passing. Nothing here, and no flag, turns
FAIL, BLOCKED or SKIP into PASS.
"""
import argparse
import base64
import datetime
import hashlib
import json
import os
import re
import sys
import tempfile

SCHEMA = "goat-federation-acceptance/v1"
PASS, FAIL, BLOCKED, SKIP, NA = "PASS", "FAIL", "BLOCKED", "SKIP", "NOT APPLICABLE"
VERDICTS = (PASS, FAIL, BLOCKED, SKIP, NA)
DIRECTIONS = ("outbound", "inbound", "both", "local")
TIERS = ("positive", "persistence", "negative", "round-trip", "spire", "precondition")
EXIT = {"PASS": 0, "FAIL": 1, "INCOMPLETE": 3}


class Malformed(ValueError):
    pass


def now():
    return datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def make_check(name, direction, verdict, reason, started, ended=None, title=None,
               tier=None, required=True, observations=None):
    """One check, validated. Raises Malformed rather than writing a record that a
    reader could misread as stronger evidence than it is."""
    if not name or any(c.isspace() for c in name):
        raise Malformed("a check needs a name without spaces, got %r" % name)
    if direction not in DIRECTIONS:
        raise Malformed("%s: direction %r is not one of %s" % (name, direction, ", ".join(DIRECTIONS)))
    if verdict not in VERDICTS:
        raise Malformed("%s: verdict %r is not one of %s" % (name, verdict, ", ".join(VERDICTS)))
    if tier is not None and tier not in TIERS:
        raise Malformed("%s: tier %r is not one of %s" % (name, tier, ", ".join(TIERS)))
    if verdict != PASS and not (reason or "").strip():
        raise Malformed("%s: a %s needs a reason" % (name, verdict))
    if verdict == NA and required:
        raise Malformed("%s: a required check cannot be NOT APPLICABLE" % name)
    return {
        "name": name,
        "title": title or name,
        "tier": tier,
        "direction": direction,
        "required": bool(required),
        "verdict": verdict,
        "reason": reason or "",
        "started_at": started,
        "ended_at": ended or now(),
        "observations": dict(observations or {}),
    }


def counts(checks):
    c = {v: 0 for v in VERDICTS}
    for ch in checks:
        c[ch["verdict"]] += 1
    return c


def overall(checks):
    """(verdict, exit status, why) of a run."""
    failed = [c["name"] for c in checks if c["verdict"] == FAIL]
    if failed:
        return "FAIL", EXIT["FAIL"], "%d check(s) failed: %s" % (len(failed), ", ".join(failed))
    required = [c for c in checks if c["required"]]
    if not required:
        return "INCOMPLETE", EXIT["INCOMPLETE"], "no required check ran, so nothing was proven"
    open_ = [c["name"] for c in required if c["verdict"] != PASS]
    if open_:
        return "INCOMPLETE", EXIT["INCOMPLETE"], "%d required check(s) did not run: %s" % (len(open_), ", ".join(open_))
    return "PASS", EXIT["PASS"], "every required check passed"


def read_obs(path):
    """`key<TAB>base64(value)` lines, as the shell writes them."""
    obs = {}
    if not path or not os.path.exists(path):
        return obs
    with open(path) as f:
        for line in f:
            line = line.rstrip("\n")
            if not line:
                continue
            key, _, b64 = line.partition("\t")
            try:
                obs[key] = base64.b64decode(b64).decode("utf-8", "replace")
            except ValueError:
                obs[key] = b64
    return obs


def read_checks(path):
    checks = []
    if os.path.exists(path):
        with open(path) as f:
            for n, line in enumerate(f, 1):
                if line.strip():
                    try:
                        checks.append(json.loads(line))
                    except ValueError as e:
                        raise Malformed("%s:%d: %s" % (path, n, e))
    return checks


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(65536), b""):
            h.update(block)
    return "sha256:" + h.hexdigest()


def bundle(checks, agreement, us, them, run_id, started, inputs, ended=None):
    verdict, code, why = overall(checks)
    ag = {}
    try:
        with open(agreement) as f:
            ag = json.load(f)
    except (OSError, ValueError):
        pass
    return {
        "schema": SCHEMA,
        "run": {"id": run_id, "started_at": started, "ended_at": ended or now(), "us": us, "them": them,
                "observed_by": us, "runner": "ops/federation/standup/acceptance.sh"},
        "agreement": {"path": os.path.abspath(agreement),
                      "sha256": sha256_file(agreement) if os.path.exists(agreement) else None,
                      "spec": ag.get("spec"), "expires": ag.get("expires")},
        # Which surfaces this run had, by name only: an address is not evidence and
        # does not belong in a file that may be handed to the partner.
        "inputs": dict(inputs),
        "counts": counts(checks),
        "verdict": verdict,
        "exit": code,
        "why": why,
        "checks": checks,
    }


def summary(b):
    lines = ["# Federation acceptance: %s <-> %s" % (b["run"]["us"], b["run"]["them"]), "",
             "Verdict: **%s** (exit %d). %s." % (b["verdict"], b["exit"], b["why"]), "",
             "Run `%s`, %s to %s, agreement `%s`." % (b["run"]["id"], b["run"]["started_at"], b["run"]["ended_at"],
                                                   b["agreement"]["sha256"]), "",
             "| Verdict | Direction | Check | What was seen |", "|---|---|---|---|"]
    for c in b["checks"]:
        reason = c["reason"].replace("|", "\\|").replace("\n", " ")
        lines.append("| %s | %s | `%s` | %s |" % (c["verdict"], c["direction"], c["name"], reason))
    lines.append("")
    lines.append("Counts: " + ", ".join("%s %d" % (k, v) for k, v in b["counts"].items()) + ".")
    return "\n".join(lines) + "\n"


def _write(path, text):
    d = os.path.dirname(path)
    os.makedirs(d, exist_ok=True)
    fd, tmp = tempfile.mkstemp(dir=d, prefix=".tmp-")
    with os.fdopen(fd, "w") as f:
        f.write(text)
    os.replace(tmp, path)


def write_bundle(b, out):
    text = json.dumps(b, indent=2, sort_keys=False) + "\n"
    safe_id = "".join(ch if ch.isalnum() or ch in "-_." else "_" for ch in b["run"]["id"])
    _write(os.path.join(out, "history", safe_id + ".json"), text)
    _write(os.path.join(out, "summary.md"), summary(b))
    _write(os.path.join(out, "result.json"), text)


def console(b):
    """The lines a person and the end-to-end test read. The counts line keeps its
    first four fields as they were; `skip` counts SKIP only."""
    c = b["counts"]
    out = ["", "acceptance-counts: pass=%d fail=%d skip=%d as=%s blocked=%d not_applicable=%d"
           % (c[PASS], c[FAIL], c[SKIP], b["run"]["us"], c[BLOCKED], c[NA])]
    if b["verdict"] == "FAIL":
        out.append("  ACCEPTANCE FAILED — %d check(s). The peering is not ready to carry anything." % c[FAIL])
    elif b["verdict"] == "INCOMPLETE":
        out.append("  ACCEPTANCE FAILED — %d passed, %d SKIPPED, %d BLOCKED. A check that did not run is not a pass;"
                   " see the lines above for why." % (c[PASS], c[SKIP], c[BLOCKED]))
    else:
        out.append("  %d passed. The peering carries what the agreement grants, refuses what it does not," % c[PASS])
        out.append("  and the publishing side can prove it without access to the counterparty.")
    return "\n".join(out)


class Unreadable(ValueError):
    pass


def rest_value(text):
    """The value of the first row a zenoh REST GET returned, as bytes; b"" when the
    surface answered with no row. A row whose stored value is empty also gives b"":
    the two read the same. Every value acceptance.sh writes is non-empty, so for its
    own keys an empty read means the key is absent; a delivery read expecting a value
    fails on either, and an absence check never sees an empty value it wrote. Raises Unreadable when it did not answer, answered
    with an error, or answered with something that is not a row list: a surface that
    cannot be read must never look like a key that is absent.

    `text` is curl's body followed by a last line holding the HTTP status
    (`curl -w '\\n%{http_code}'`); curl prints 000 when it could not connect."""
    body, _, code = text.rstrip("\n").rpartition("\n")
    code = code.strip()
    if code != "200":
        raise Unreadable("HTTP %s" % (code or "<none>") if code != "000" else "no answer (connection failed or timed out)")
    try:
        rows = json.loads(body)
    except ValueError:
        raise Unreadable("the answer is not JSON: %r" % body[:80])
    if not isinstance(rows, list):
        raise Unreadable("the answer is not a list of rows: %r" % body[:80])
    if not rows:
        return b""
    row = rows[0]
    if not isinstance(row, dict) or "value" not in row:
        raise Unreadable("a row without a value: %r" % body[:80])
    value, encoding = row["value"], str(row.get("encoding") or "")
    if not isinstance(value, str):
        return json.dumps(value, separators=(",", ":")).encode()
    if encoding.startswith("text/") or encoding in ("zenoh/string", "application/json"):
        return value.encode()
    # Anything else comes back base64, and nothing in the row says so for certain.
    try:
        return base64.b64decode(value, validate=True)
    except ValueError:
        return value.encode()


def rest_rows(text):
    """Every row a zenoh REST GET returned, as a list. Raises Unreadable on the same
    answers rest_value refuses."""
    body, _, code = text.rstrip("\n").rpartition("\n")
    code = code.strip()
    if code != "200":
        raise Unreadable("HTTP %s" % (code or "<none>") if code != "000" else "no answer (connection failed or timed out)")
    try:
        rows = json.loads(body)
    except ValueError:
        raise Unreadable("the answer is not JSON: %r" % body[:80])
    if not isinstance(rows, list) or not all(isinstance(r, dict) and "key" in r for r in rows):
        raise Unreadable("the answer is not a list of rows: %r" % body[:80])
    return rows


def key_matches(expr, key):
    """Whether the key expression `expr` matches the single key `key`: `*` is one
    chunk, `**` any number of chunks including none. Enough for subscriber names."""
    e, k = expr.split("/"), key.split("/")

    def m(i, j):
        if i == len(e):
            return j == len(k)
        if e[i] == "**":
            return any(m(i + 1, n) for n in range(j, len(k) + 1))
        if j >= len(k):
            return False
        pat = "^" + ".*".join(re.escape(p) for p in e[i].split("*")) + "$"
        return re.match(pat, k[j]) is not None and m(i + 1, j + 1)

    return m(0, 0)


SUBSCRIBER_KEY = re.compile(r"^@/([^/]+)/router/subscriber/(.+)$")


def persistence_scope(key, router_rows, subscriber_rows):
    """Who on the receiving core would take a sample published on `key`.

    `router_rows` answers `@/*/router`: it is empty when the admin space is closed,
    and an empty subscriber list would then read as "no storage", so it is checked
    first. `subscriber_rows` answers `@/*/router/subscriber/<key>`; each row names a
    subscribed key expression and its sources by zenoh id, in the shape zenoh 1.9
    prints (`{"clients": [...], "peers": [...], "routers": [...]}`).

    A storage runs inside the router, so its subscriber is sourced from the router's
    own id. Any other client or peer subscribed to the key is a session someone
    opened, a diagnostic interest: while it exists, a sample can reach the core
    because of it, and a store whose own interest is refused would look healthy.

    Returns (outcome, reason, details) with outcome one of
      ready       a store of the router itself subscribes to the key, nothing else does
      no-storage  no subscriber of the router itself matches the key
      diagnostic  a client or peer session also subscribes to it
      closed      the admin space answered nothing
    """
    routers = [r for r in router_rows if re.match(r"^@/[^/]+/router$", str(r.get("key", "")))]
    if not routers:
        return ("closed", "the receiving core's admin space answered nothing; it needs adminspace read permission",
                {"key": key})
    own = {str(r["key"]).split("/")[1] for r in routers}
    local, diagnostic, remote = [], [], []
    for row in subscriber_rows:
        m = SUBSCRIBER_KEY.match(str(row.get("key", "")))
        if not m or not key_matches(m.group(2), key):
            continue
        zid, expr, src = m.group(1), m.group(2), row.get("value") or {}
        if not isinstance(src, dict):
            src = {}
        sessions = [z for kind in ("clients", "peers") for z in (src.get(kind) or [])]
        if zid in own and zid in sessions:
            local.append(expr)
        for z in sessions:
            if z not in own:
                diagnostic.append({"expr": expr, "session": z})
        for z in src.get("routers") or []:
            if z not in own:
                remote.append({"expr": expr, "router": z})
    details = {"key": key, "store_subscriptions": sorted(set(local)), "diagnostic": diagnostic,
               "remote_routers": remote}
    if not local:
        return ("no-storage", "no store on the receiving core subscribes to %s" % key, details)
    if diagnostic:
        return ("diagnostic", "a session other than the core's own stores subscribes to %s: %s" % (
            key, ", ".join("%s (%s)" % (d["expr"], d["session"]) for d in diagnostic)), details)
    return ("ready", "only the core's own store (%s) subscribes to %s" % (", ".join(details["store_subscriptions"]), key),
            details)


SCOPE_EXIT = {"ready": 0, "no-storage": 1, "closed": 4, "diagnostic": 5}


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("record")
    r.add_argument("checks")
    r.add_argument("--name", required=True)
    r.add_argument("--direction", required=True)
    r.add_argument("--verdict", required=True)
    r.add_argument("--reason", default="")
    r.add_argument("--started", required=True)
    r.add_argument("--title")
    r.add_argument("--tier")
    r.add_argument("--optional", action="store_true")
    r.add_argument("--obs-file")
    f = sub.add_parser("finalize")
    f.add_argument("checks")
    f.add_argument("--out", required=True)
    f.add_argument("--agreement", required=True)
    f.add_argument("--us", required=True)
    f.add_argument("--them", required=True)
    f.add_argument("--run-id", required=True)
    f.add_argument("--started", required=True)
    f.add_argument("--inputs", nargs="*", default=[])
    sub.add_parser("rest-value")
    ps = sub.add_parser("persistence-scope")
    ps.add_argument("--key", required=True)
    ps.add_argument("--router", required=True)
    ps.add_argument("--subscribers", required=True)
    a = ap.parse_args(argv)
    if a.cmd == "rest-value":
        try:
            sys.stdout.buffer.write(rest_value(sys.stdin.read()))
            return 0
        except Unreadable as e:
            print(str(e), file=sys.stderr)
            return 4
    if a.cmd == "persistence-scope":
        # Prints one JSON line (outcome, reason, details); the exit status is the outcome.
        try:
            router = rest_rows(open(a.router).read())
            subs = rest_rows(open(a.subscribers).read())
        except (OSError, Unreadable) as e:
            print(json.dumps({"outcome": "unreadable", "reason": "the receiving core's admin space: %s" % e}))
            return 4
        outcome, reason, details = persistence_scope(a.key, router, subs)
        print(json.dumps({"outcome": outcome, "reason": reason, "details": details}, sort_keys=True))
        return SCOPE_EXIT[outcome]
    try:
        if a.cmd == "record":
            ch = make_check(a.name, a.direction, a.verdict, a.reason, a.started, title=a.title, tier=a.tier,
                            required=not a.optional, observations=read_obs(a.obs_file))
            with open(a.checks, "a") as fh:
                fh.write(json.dumps(ch, sort_keys=False) + "\n")
            return 0
        inputs = {}
        for kv in a.inputs:
            k, _, v = kv.partition("=")
            inputs[k] = v == "1"
        b = bundle(read_checks(a.checks), a.agreement, a.us, a.them, a.run_id, a.started, inputs)
        write_bundle(b, a.out)
        print(console(b))
        print("  evidence: %s" % os.path.join(a.out, "result.json"))
        return b["exit"]
    except Malformed as e:
        print("acceptance evidence: %s" % e, file=sys.stderr)
        return 2
    except OSError as e:
        print("acceptance evidence: the bundle could not be written: %s" % e, file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
