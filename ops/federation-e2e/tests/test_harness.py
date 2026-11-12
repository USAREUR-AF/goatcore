#!/usr/bin/env python3
"""The federation e2e harness, checked without Docker (CI: check-federation-e2e-harness.sh).

    python3 ops/federation-e2e/tests/test_harness.py

1. The catalog: unique scenario ids, every unit it names exists as units/<unit>.sh, every
   scenario has a pass condition, runs_where is ci or local, every check names a step.
2. The outcome rules of lib/verdict.py on synthetic records: a passing step is PASS, a failing
   one FAIL; a unit that stopped makes its later checks BLOCKED; a unit not run is SKIP; a
   mark carries its outcome and reason; a required BLOCKED or SKIP never exits 0; a scenario
   whose required checks are all NOT APPLICABLE is NOT APPLICABLE; the result validates
   against result.schema.json.
3. lib/handoff.py on the two-driver contract's own hand-off object: it reads a stop's hand-off
   and an edge run's way back, refuses a confirm code that is not the result's, fills only
   whole-element tokens, and finds each command quoted in the person's text.
Exit 0 when all hold; prints one line per failure otherwise.
"""
import json
import os
import subprocess
import sys
import tempfile

import yaml

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
FAIL = []


def read_json(path):
    with open(path, encoding="utf-8") as fh:
        return json.load(fh)


def read_yaml(path):
    with open(path, encoding="utf-8") as fh:
        return yaml.safe_load(fh)


def write_text(path, text):
    with open(path, "w", encoding="utf-8") as fh:
        fh.write(text)


def check(cond, what):
    if not cond:
        FAIL.append(what)


def catalog_checks():
    cat = read_yaml(os.path.join(ROOT, "scenarios", "catalog.yaml"))
    ids = [s["id"] for s in cat["scenarios"]]
    check(len(ids) == len(set(ids)), "catalog: duplicate scenario ids")
    for s in cat["scenarios"]:
        check(bool((s.get("pass_condition") or "").strip()), f"catalog: {s['id']} has no pass condition")
        check(s.get("runs_where", "local") in ("ci", "local"), f"catalog: {s['id']} runs_where is not ci or local")
        units = s.get("unit")
        units = list(units.values()) if isinstance(units, dict) else ([units] if units else [])
        if not s.get("not_built"):
            check(bool(units), f"catalog: {s['id']} is built but names no unit")
        for u in units:
            check(os.path.exists(os.path.join(ROOT, "units", f"{u}.sh")), f"catalog: {s['id']} names unit {u}, no units/{u}.sh")
        for c in s.get("checks", []):
            check(bool(c.get("step")), f"catalog: {s['id']}.{c.get('id')} names no step")


def write(d, name, rows):
    with open(os.path.join(d, name), "w") as fh:
        for r in rows:
            fh.write(json.dumps(r) + "\n")


def step(n, name, status):
    return {"n": n, "name": name, "status": status, "command": ["true"], "exit": 0 if status == "pass" else 1,
            "expected_exit": 0, "duration_s": 0.1, "started": "2026-01-01T00:00:00Z", "log": f"{n:02d}-{name}.log"}


def verdict_checks():
    cat = {"schema": "fed-e2e-catalog/1", "params": {"A": "xaa"}, "scenarios": [
        {"id": "all-pass", "title": "t", "unit": "u1", "pass_condition": "p", "checks": [{"id": "a", "step": "s-{A}-a"}]},
        {"id": "one-fails", "title": "t", "unit": "u1", "pass_condition": "p", "checks": [{"id": "a", "step": "s-{A}-a"}, {"id": "b", "step": "s-b"}]},
        {"id": "stopped", "title": "t", "unit": "u2", "pass_condition": "p", "checks": [{"id": "c", "step": "s-c"}]},
        {"id": "not-run", "title": "t", "unit": "u3", "pass_condition": "p", "checks": [{"id": "d", "step": "s-d"}]},
        {"id": "excluded", "title": "t", "unit": "u1", "pass_condition": "p", "checks": [{"id": "e", "step": "s-e"}]},
        {"id": "optional-fail", "title": "t", "unit": "u1", "pass_condition": "p", "checks": [{"id": "a", "step": "s-{A}-a"}, {"id": "f", "step": "s-b", "required": False}]},
        {"id": "mark-beats-pass", "title": "t", "unit": "u1", "pass_condition": "p", "checks": [{"id": "g", "step": "s-xaa-a"}]},
        {"id": "fail-beats-mark", "title": "t", "unit": "u1", "pass_condition": "p", "checks": [{"id": "h", "step": "s-b"}]},
    ]}
    with tempfile.TemporaryDirectory() as t:
        cp = os.path.join(t, "catalog.yaml")
        write_text(cp, yaml.safe_dump(cat))
        u1, u2 = os.path.join(t, "u1"), os.path.join(t, "u2")
        os.makedirs(u1); os.makedirs(u2)
        write(u1, "report.jsonl", [step(1, "s-xaa-a", "pass"), step(2, "s-b", "fail")])
        write(u1, "marks.jsonl", [{"check": "excluded.e", "outcome": "NOT APPLICABLE", "reason": "agreement offers no such flow"},
                                  {"check": "mark-beats-pass.g", "outcome": "NOT APPLICABLE", "reason": "nothing to exercise on this host"},
                                  {"check": "fail-beats-mark.h", "outcome": "BLOCKED", "reason": "an excuse"}])
        write_text(os.path.join(u2, "stopped.json"), json.dumps({"reason": "probe did not come up"}))
        out = os.path.join(t, "r.json")
        p = subprocess.run([sys.executable, os.path.join(ROOT, "lib", "verdict.py"), "--catalog", cp, "--record", u1, "--record", u2,
                            "--ref-name", "fixture", "--ref-sha", "0" * 40, "--avenue", "goatbuilder", "--out", out],
                           capture_output=True, text=True)
        check(os.path.exists(out), f"verdict wrote no result: {p.stderr[-300:]}")
        if not os.path.exists(out):
            return
        r = read_json(out)
        by = {s["id"]: s for s in r["scenarios"]}
        want = {"all-pass": "PASS", "one-fails": "FAIL", "stopped": "BLOCKED", "not-run": "SKIP", "excluded": "NOT APPLICABLE", "optional-fail": "PASS",
                "mark-beats-pass": "NOT APPLICABLE", "fail-beats-mark": "FAIL"}
        for k, v in want.items():
            check(by[k]["outcome"] == v, f"verdict: {k} is {by[k]['outcome']}, want {v}")
        check(by["excluded"]["checks"][0]["reason"] == "agreement offers no such flow", "verdict: a mark lost its reason")
        check(p.returncode == 1 and r["verdict"]["exit"] == 1, "verdict: a required FAIL, BLOCKED or SKIP exited 0")
        # Only passing and excluded scenarios: exit 0.
        p2 = subprocess.run([sys.executable, os.path.join(ROOT, "lib", "verdict.py"), "--catalog", cp, "--record", u1, "--only", "all-pass,excluded,optional-fail",
                             "--ref-name", "fixture", "--ref-sha", "0" * 40, "--avenue", "goatbuilder", "--out", out], capture_output=True, text=True)
        check(p2.returncode == 0, "verdict: PASS and NOT APPLICABLE only did not exit 0")
        # A ref that did not build: every scenario BLOCKED with the reason, and a non-zero exit.
        p4 = subprocess.run([sys.executable, os.path.join(ROOT, "lib", "verdict.py"), "--catalog", cp, "--record", u1,
                             "--blocked-all", "the tools did not build", "--harness-fp", "abc123",
                             "--ref-name", "fixture", "--ref-sha", "0" * 40, "--avenue", "goatbuilder", "--out", out + ".4"], capture_output=True, text=True)
        r4 = read_json(out + ".4") if os.path.exists(out + ".4") else {"scenarios": []}
        check(p4.returncode == 1 and r4["scenarios"] and all(x["outcome"] == "BLOCKED" for x in r4["scenarios"]), "verdict: --blocked-all did not make every scenario BLOCKED")
        check(r4.get("harness", {}).get("fingerprint") == "abc123", "verdict: the harness fingerprint is not in the result")
        check("directory" not in r4, "verdict: a result on the pinned directory carries a directory field")
        # A directory built from a saltlick commit is named in the result.
        p5 = subprocess.run([sys.executable, os.path.join(ROOT, "lib", "verdict.py"), "--catalog", cp, "--record", u1,
                             "--directory-ref", "origin/example", "--directory-sha", "1" * 40,
                             "--ref-name", "fixture", "--ref-sha", "0" * 40, "--avenue", "goatbuilder", "--out", out + ".5"], capture_output=True, text=True)
        r5 = read_json(out + ".5") if os.path.exists(out + ".5") else {}
        check(r5.get("directory") == {"ref": "origin/example", "sha": "1" * 40}, "verdict: --directory-ref is not in the result: " + p5.stderr[-300:])
        # A mark may not say PASS or FAIL.
        write(u2, "marks.jsonl", [{"check": "stopped.c", "outcome": "PASS", "reason": "trust me"}])
        p3 = subprocess.run([sys.executable, os.path.join(ROOT, "lib", "verdict.py"), "--catalog", cp, "--record", u2,
                             "--ref-name", "fixture", "--ref-sha", "0" * 40, "--avenue", "goatbuilder", "--out", out + ".3"], capture_output=True, text=True)
        check(p3.returncode != 0 and "may say only" in p3.stderr + p3.stdout, "verdict: a PASS mark was accepted")
        try:
            import jsonschema
            jsonschema.validate(read_json(out), read_json(os.path.join(ROOT, "result.schema.json")))
        except ImportError:
            FAIL.append("jsonschema is not installed")


def handoff_checks():
    """lib/handoff.py on the contract's own hand-off object (federation-two-drivers.md)."""
    h = os.path.join(ROOT, "lib", "handoff.py")
    run = lambda *a: subprocess.run([sys.executable, h, *a], capture_output=True, text=True)
    with tempfile.TemporaryDirectory() as d:
        stop = {"ok": True, "handoff": {"phase": 3, "code": "GB-NNN", "run_on": "edge",
                "command": ["goatbuilder", "federate", "xab", "--side", "edge", "--from", "3", "--to", "3",
                            "--expect-commit", "c" * 40],
                "commit": "c" * 40, "carry": [], "result": "handoff/3/edge-half.json",
                "import": ["goatbuilder", "federate", "import", "xab", "3", "{result}", "--sha256", "{code}"],
                "then": ["goatbuilder", "federate", "xab", "--from", "3"]}}
        write_text(os.path.join(d, "stop.json"), json.dumps(stop))
        r = run("next", os.path.join(d, "stop.json"))
        check(r.returncode == 0 and json.loads(r.stdout)["run_on"] == "edge", f"handoff next reads the stop: {r.stderr.strip()}")
        nocommit = json.loads(json.dumps(stop)); del nocommit["handoff"]["commit"]
        write_text(os.path.join(d, "nocommit.json"), json.dumps(nocommit))
        r = run("next", os.path.join(d, "nocommit.json"))
        check(r.returncode == 2, "handoff next refuses an edge hand-off that names no commit")
        badcarry = json.loads(json.dumps(stop)); badcarry["handoff"]["carry"] = [{"file": "agreement.json", "sha256": "a" * 64}]
        write_text(os.path.join(d, "badcarry.json"), json.dumps(badcarry))
        r = run("next", os.path.join(d, "badcarry.json"))
        check(r.returncode == 2, "handoff next refuses a carry entry without in: checkout|work")
        okcarry = json.loads(json.dumps(stop)); okcarry["handoff"]["carry"] = [{"file": "agreement.json", "in": "work", "sha256": "a" * 64}]
        write_text(os.path.join(d, "okcarry.json"), json.dumps(okcarry))
        r = run("next", os.path.join(d, "okcarry.json"))
        check(r.returncode == 0, f"handoff next reads a work carry entry: {r.stderr.strip()}")
        write_text(os.path.join(d, "none.json"), json.dumps({"ok": True}))
        r = run("next", os.path.join(d, "none.json"))
        check(r.returncode == 0 and json.loads(r.stdout)["run_on"] is None, "handoff next: a report without a hand-off is none")
        sha = "ab12" * 16
        code = " ".join(sha[:32][i:i + 4] for i in range(0, 32, 4))
        way = {"handoff": {"phase": 3, "code": "GB-NNN", "run_on": "controller", "command": None,
               "result_path": "/var/lib/goat-federation/xab/handoff/3/edge-half.json", "result_sha256": sha,
               "confirm_code": code, "import": ["goatbuilder", "federate", "import", "xab", "3", "{result}", "--sha256", code],
               "then": ["goatbuilder", "federate", "xab", "--from", "3"]}}
        write_text(os.path.join(d, "way.json"), json.dumps(way))
        r = run("wayback", os.path.join(d, "way.json"))
        check(r.returncode == 0 and json.loads(r.stdout)["confirm_code"] == code, f"handoff wayback reads the edge's code: {r.stderr.strip()}")
        way["handoff"]["confirm_code"] = "0000 " * 8
        write_text(os.path.join(d, "bad.json"), json.dumps(way))
        r = run("wayback", os.path.join(d, "bad.json"))
        check(r.returncode == 2, "handoff wayback refuses a confirm code that is not the result's")
        r = run("fill", json.dumps(stop["handoff"]["import"]), "--result", "/tmp/r.json", "--code", code)
        check(r.returncode == 0 and json.loads(r.stdout)[5:] == ["/tmp/r.json", "--sha256", code], f"handoff fill replaces whole-element tokens: {r.stderr.strip()}")
        r = run("fill", json.dumps(["x", "--file={result}"]), "--result", "/tmp/r.json")
        check(r.returncode == 2, "handoff fill refuses a token inside an element")
        write_text(os.path.join(d, "text.txt"), "continue on the edge driver:\n  goatbuilder federate xab --side edge --from 3 --to 3 --expect-commit " + "c" * 40 + "\n")
        r = run("text-holds", os.path.join(d, "text.txt"), json.dumps(stop["handoff"]["command"]))
        check(r.returncode == 0, f"handoff text-holds finds the quoted argv: {r.stderr.strip()}")
        r = run("text-holds", os.path.join(d, "text.txt"), json.dumps(stop["handoff"]["then"]))
        check(r.returncode == 2, "handoff text-holds refuses a text that lacks the argv")


def fwdeclared_checks():
    """lib/fwdeclared.py: the ports a goat-host-firewall rule set admits from one source."""
    ruleset = """table inet goat_host_firewall
delete table inet goat_host_firewall
table inet goat_host_firewall {
  chain input {
    type filter hook input priority filter; policy drop;
    iifname "lo" accept comment "loopback"
    udp sport 67 udp dport 68 accept comment "dhcp-client"
    iifname "br-*" accept comment "container-bridges"
    ip saddr { 198.51.100.10/32 } tcp dport { 22 } accept comment "ssh-management"
    ip saddr { 192.0.2.10 } udp dport { 51820 } accept comment "wg-fed0/tunnel"
    iifname "wg-fed0" ip saddr { 169.254.10.0 } tcp dport { 7448 } accept comment "wg-fed0/edge-router"
    tcp dport { 7447 } accept comment "fabric-tls"
    counter drop comment "default-deny"
  }
  chain published_new {
    ip saddr { 203.0.113.5/32 } meta l4proto tcp ct original proto-dst { 9999 } accept comment "other"
  }
}
"""
    with tempfile.TemporaryDirectory() as d:
        f = os.path.join(d, "ruleset.nft")
        write_text(f, ruleset)

        def run(saddr, iif):
            p = subprocess.run([sys.executable, os.path.join(ROOT, "lib", "fwdeclared.py"), "--ruleset", f, "--saddr", saddr, "--iif", iif],
                               capture_output=True, text=True)
            return json.loads(p.stdout) if p.returncode == 0 else None

        r = run("198.51.100.10", "eth0")
        check(r and r["tcp"] == [22, 7447] and r["udp"] == [], f"fwdeclared: management source got {r and (r['tcp'], r['udp'])}")
        r = run("169.254.10.0", "wg-fed0")
        check(r and r["tcp"] == [7447, 7448], f"fwdeclared: the far link address on the tunnel got {r and r['tcp']}")
        r = run("169.254.10.0", "eth0")
        check(r and r["tcp"] == [7447], "fwdeclared: a rule bound to the tunnel interface admitted another interface")
        r = run("192.0.2.10", "eth1")
        check(r and r["udp"] == [51820] and r["by_port"].get("udp/51820") == ["wg-fed0/tunnel"], "fwdeclared: the tunnel port from the partner endpoint")
        r = run("203.0.113.5", "eth0")
        check(r and r["tcp"] == [7447], "fwdeclared: a rule outside chain input or a fixed expression was counted")
        write_text(f, "table inet other {}\n")
        check(run("198.51.100.10", "eth0") is None, "fwdeclared: a file without the goat_host_firewall table was read as a rule set")


catalog_checks()
verdict_checks()
fwdeclared_checks()
handoff_checks()
for f in FAIL:
    print(f"FAIL {f}")
print("ok: federation e2e harness checks" if not FAIL else f"{len(FAIL)} harness check(s) failed")
sys.exit(1 if FAIL else 0)
