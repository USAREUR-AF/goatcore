#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
#
# ladder_gate.py — a ref reaches a higher rung of the release ladder only after
# the rung below runs it (ADR 1120; error GB-373).
#
# The ladder (ops/release/ladder.yaml) orders the live rungs, each a fleet-pins
# goatnet: dev, then preprod, then prod. Before a deploy touches a host on a rung
# above the lowest, this gate requires the exact image digest being deployed to
# be the one the next rung down pins (its rendered group_vars/<member>/pins.yml).
# deploy-component.sh then confirms, live, that the lower rung's hosts converged
# on it (its own --verify-only read of that rung's deploy ledgers).
#
# Two recorded exceptions, and only two:
#   - an open hotfix record (class: hotfix, ops/release/records/<component>/) for
#     this component and digest: graduate.py hotfix / fastlane.sh (ADR 1127)
#     moved every rung at once on purpose and owes the backfill;
#   - --skip-rung with BOTH a reason and an operator ack; the caller writes them
#     into the deploy ledger row (ladder_skip).
#
# A rung that does not pin the component is passed over and the next rung down
# decides. When no lower rung pins it at all the gate refuses (fail-closed): a
# component no lower rung runs has never been seen working anywhere.
#
# Usage:
#   ladder_gate.py component --repo R --component C --goatnet G --ref REF
#                            [--skip-reason TEXT --skip-ack NAME]
#     exit 0  pass (no rung below, not on the ladder, or a recorded exception)
#     exit 3  pass statically; the caller must confirm convergence on the
#             goatnet printed as lower_goatnet
#     exit 1  refused (GB-373); the message says what to run first
#   ladder_gate.py play --repo R [--skip-reason TEXT --skip-ack NAME] -- <ansible-playbook args>
#     judges every fleet component the play deploys to a member group named in
#     its arguments, using an -e override of the component's image variable when
#     one is given. exit 0 pass, 1 refused. Live convergence is not read here:
#     the play mode is the guard for direct plays, deploy-component.sh is the
#     path that also reads the lower rung's ledgers.
#   Both print one JSON object per decision on stdout.

import argparse
import json
import os
import re
import sys

try:
    import yaml
except ImportError:  # pragma: no cover - the deploy toolchain installs PyYAML
    sys.exit("ladder_gate.py needs PyYAML")

CODE = "GB-373"
NON_MUTATING = {"--list-tasks", "--list-hosts", "--list-tags", "--syntax-check", "--check", "-C"}
VALUE_FLAGS = {"-i", "--inventory", "-e", "--extra-vars", "-l", "--limit", "-t", "--tags",
               "--skip-tags", "-u", "--user", "-f", "--forks", "--vault-id",
               "--vault-password-file", "-M", "--module-path", "--become-user", "-c",
               "--connection", "-T", "--timeout", "--private-key", "--ssh-common-args",
               "--start-at-task", "--become-method"}


def digest_of(ref):
    ref = (ref or "").strip().strip('"').strip("'")
    return ref.split("@", 1)[1] if "@" in ref else ref


def _load(path):
    try:
        with open(path) as f:
            return yaml.safe_load(f) or {}
    except FileNotFoundError:
        return {}


def live_rungs(repo):
    ladder = _load(os.path.join(repo, "ops/release/ladder.yaml"))
    return [r for r in (ladder.get("rungs") or []) if r.get("status") == "live" and r.get("goatnet")]


def fleet(repo):
    return _load(os.path.join(repo, "ops/pins/fleet-pins.yaml"))


def pinned_digests(repo, pins, comp, goatnet):
    """Digests the goatnet's member groups pin for the component (empty: not pinned there)."""
    var = ((pins.get("components") or {}).get(comp) or {}).get("var")
    members = ((pins.get("goatnets") or {}).get(goatnet) or {}).get("members") or []
    out = set()
    if not var:
        return out
    pat = re.compile(r"^%s:\s*(.+?)\s*$" % re.escape(var))
    for m in members:
        p = os.path.join(repo, "ops/ansible/group_vars", m, "pins.yml")
        try:
            with open(p) as f:
                for line in f:
                    mm = pat.match(line)
                    if mm:
                        out.add(digest_of(mm.group(1)))
        except FileNotFoundError:
            continue
    return out


def hotfix_record(repo, comp, digest):
    d = os.path.join(repo, "ops/release/records", comp)
    if not os.path.isdir(d):
        return None
    for name in sorted(os.listdir(d)):
        if not name.endswith(".yaml"):
            continue
        try:
            rec = _load(os.path.join(d, name))
        except yaml.YAMLError:
            continue
        if rec.get("class") == "hotfix" and rec.get("state") != "abandoned" \
                and digest_of(rec.get("candidate_ref")) == digest:
            return os.path.join("ops/release/records", comp, name)
    return None


def _short(digest):
    return digest.split(":", 1)[-1][:12] if digest else "(none)"


def decide(repo, comp, goatnet, digest, skip_reason="", skip_ack="", pins=None):
    pins = pins if pins is not None else fleet(repo)
    rungs = live_rungs(repo)
    base = {"component": comp, "goatnet": goatnet, "digest": digest, "code": CODE}
    idx = next((i for i, r in enumerate(rungs) if r["goatnet"] == goatnet), None)
    if idx is None:
        return dict(base, decision="pass", message="ladder: goatnet %s is not a live rung of the ladder; nothing to enforce" % goatnet)
    rung = rungs[idx]["name"]
    base["rung"] = rung
    if idx == 0:
        return dict(base, decision="pass", message="ladder: %s is the lowest rung (%s); nothing below it" % (goatnet, rung))
    if skip_reason or skip_ack:
        if not skip_reason.strip():
            return dict(base, decision="refuse", message="%s ladder: --skip-rung needs --reason \"<why the rung below is skipped>\"" % CODE)
        if not skip_ack.strip():
            return dict(base, decision="refuse", message="%s ladder: --skip-rung needs --ack \"<operator>\": skipping a rung is attributed" % CODE)
        return dict(base, decision="pass", exception="skip-rung", reason=skip_reason, ack=skip_ack,
                    message="ladder: rung below %s SKIPPED for %s %s — reason: %s; ack: %s (recorded in the ledger row)"
                    % (rung, comp, _short(digest), skip_reason, skip_ack))
    rec = hotfix_record(repo, comp, digest)
    if rec:
        return dict(base, decision="pass", exception="hotfix-record", record=rec,
                    message="ladder: %s %s is an open hotfix (%s); every rung moves at once and the backfill is owed"
                    % (comp, _short(digest), rec))
    passed_over = []
    for lower in reversed(rungs[:idx]):
        have = pinned_digests(repo, pins, comp, lower["goatnet"])
        if not have:
            passed_over.append(lower["goatnet"])
            continue
        if digest in have:
            return dict(base, decision="lower", lower_rung=lower["name"], lower_goatnet=lower["goatnet"],
                        message="ladder: %s %s is pinned on rung %s (%s); confirming it converged there"
                        % (comp, _short(digest), lower["name"], lower["goatnet"]))
        return dict(base, decision="refuse", lower_rung=lower["name"], lower_goatnet=lower["goatnet"],
                    message=("%s ladder: %s %s is not on rung %s (goatnet %s), the rung below %s (%s); it pins %s there. "
                             "Put it there first: move its pin on %s and run ops/scripts/deploy-component.sh %s --goatnet %s; "
                             "or take a recorded exception: an open hotfix record (graduate.py hotfix / fastlane.sh), "
                             "or --skip-rung --reason \"<why>\" --ack \"<operator>\"")
                    % (CODE, comp, _short(digest), lower["name"], lower["goatnet"], rung, goatnet,
                       ", ".join(sorted(_short(h) for h in have)), lower["goatnet"], comp, lower["goatnet"]))
    return dict(base, decision="refuse",
                message=("%s ladder: no rung below %s (%s) pins %s (passed over: %s), so %s has not run anywhere first. "
                         "Deploy it on a lower rung first, or take a recorded exception: an open hotfix record, "
                         "or --skip-rung --reason \"<why>\" --ack \"<operator>\"")
                % (CODE, rung, goatnet, comp, ", ".join(passed_over) or "none", _short(digest)))


def _extra_vars(args):
    out = {}
    for i, a in enumerate(args):
        val = None
        if a in ("-e", "--extra-vars") and i + 1 < len(args):
            val = args[i + 1]
        elif a.startswith("--extra-vars="):
            val = a.split("=", 1)[1]
        if val is None:
            continue
        if val.startswith("@"):
            data = _load(val[1:])
            if isinstance(data, dict):
                out.update(data)
            continue
        s = val.strip()
        if s.startswith("{"):
            try:
                out.update(json.loads(s))
                continue
            except ValueError:
                pass
        for tok in s.split():
            if "=" in tok:
                k, v = tok.split("=", 1)
                out[k] = v.strip('"').strip("'")
    return out


def _playbook(args):
    for i, a in enumerate(args):
        if a.endswith((".yml", ".yaml")) and not a.startswith("@") and (i == 0 or args[i - 1] not in VALUE_FLAGS):
            return os.path.basename(a)
    return None


def _limit_terms(args):
    terms = []
    for i, a in enumerate(args):
        val = args[i + 1] if a in ("-l", "--limit") and i + 1 < len(args) else (a.split("=", 1)[1] if a.startswith("--limit=") else None)
        if val:
            for t in re.split(r"[:,]", val):
                t = t.strip()
                if t and not t.startswith(("!", "&")):
                    terms.append(t)
    return terms


def play_decisions(repo, args, skip_reason="", skip_ack=""):
    if any(a in NON_MUTATING for a in args):
        return []
    play = _playbook(args)
    if not play:
        return []
    pins = fleet(repo)
    comps = pins.get("components") or {}
    goatnets = pins.get("goatnets") or {}
    ev = _extra_vars(args)
    member_of = {m: g for g, spec in goatnets.items() for m in (spec.get("members") or [])}
    groups = []
    for v in list(ev.values()) + _limit_terms(args):
        for t in re.split(r"[:,]", str(v)):
            t = t.strip()
            if t in member_of and t not in groups:
                groups.append(t)
    out = []
    for name, c in comps.items():
        names = {c.get("deploy_playbook"), "%s-deploy.yml" % (c.get("ledger_component") or name),
                 "%s-deploy.yml" % name, "%s.yml" % name}
        if play not in names or not c.get("var"):
            continue
        for grp in groups:
            if c["var"] in ev:
                digest = digest_of(str(ev[c["var"]]))
            else:
                p = os.path.join(repo, "ops/ansible/group_vars", grp, "pins.yml")
                digest = ""
                try:
                    with open(p) as f:
                        for line in f:
                            if line.startswith(c["var"] + ":"):
                                digest = digest_of(line.split(":", 1)[1])
                except FileNotFoundError:
                    pass
            if not digest:
                continue
            d = decide(repo, name, member_of[grp], digest, skip_reason, skip_ack, pins=pins)
            d["group"] = grp
            if d["decision"] == "lower":
                d["decision"] = "pass"
                d["message"] += " (a direct play does not read the lower rung's ledgers; deploy-component.sh does)"
            out.append(d)
    return out


def main(argv):
    ap = argparse.ArgumentParser(prog="ladder_gate.py")
    sub = ap.add_subparsers(dest="mode", required=True)
    c = sub.add_parser("component")
    c.add_argument("--repo", required=True)
    c.add_argument("--component", required=True)
    c.add_argument("--goatnet", required=True)
    c.add_argument("--ref", required=True)
    c.add_argument("--skip-reason", default="")
    c.add_argument("--skip-ack", default="")
    p = sub.add_parser("play")
    p.add_argument("--repo", required=True)
    p.add_argument("--skip-reason", default="")
    p.add_argument("--skip-ack", default="")
    p.add_argument("rest", nargs=argparse.REMAINDER)
    a = ap.parse_args(argv)
    if a.mode == "component":
        d = decide(a.repo, a.component, a.goatnet, digest_of(a.ref), a.skip_reason, a.skip_ack)
        print(json.dumps(d))
        return {"pass": 0, "lower": 3, "refuse": 1}[d["decision"]]
    rest = a.rest[1:] if a.rest[:1] == ["--"] else a.rest
    ds = play_decisions(a.repo, rest, a.skip_reason, a.skip_ack)
    for d in ds:
        print(json.dumps(d))
    return 1 if any(d["decision"] == "refuse" for d in ds) else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
