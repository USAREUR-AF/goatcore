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


def check(cond, what):
    if not cond:
        FAIL.append(what)


def catalog_checks():
    cat = yaml.safe_load(open(os.path.join(ROOT, "scenarios", "catalog.yaml")))
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
        yaml.safe_dump(cat, open(cp, "w"))
        u1, u2 = os.path.join(t, "u1"), os.path.join(t, "u2")
        os.makedirs(u1); os.makedirs(u2)
        write(u1, "report.jsonl", [step(1, "s-xaa-a", "pass"), step(2, "s-b", "fail")])
        write(u1, "marks.jsonl", [{"check": "excluded.e", "outcome": "NOT APPLICABLE", "reason": "agreement offers no such flow"},
                                  {"check": "mark-beats-pass.g", "outcome": "NOT APPLICABLE", "reason": "nothing to exercise on this host"},
                                  {"check": "fail-beats-mark.h", "outcome": "BLOCKED", "reason": "an excuse"}])
        json.dump({"reason": "probe did not come up"}, open(os.path.join(u2, "stopped.json"), "w"))
        out = os.path.join(t, "r.json")
        p = subprocess.run([sys.executable, os.path.join(ROOT, "lib", "verdict.py"), "--catalog", cp, "--record", u1, "--record", u2,
                            "--ref-name", "fixture", "--ref-sha", "0" * 40, "--avenue", "goatbuilder", "--out", out],
                           capture_output=True, text=True)
        check(os.path.exists(out), f"verdict wrote no result: {p.stderr[-300:]}")
        if not os.path.exists(out):
            return
        r = json.load(open(out))
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
        r4 = json.load(open(out + ".4")) if os.path.exists(out + ".4") else {"scenarios": []}
        check(p4.returncode == 1 and r4["scenarios"] and all(x["outcome"] == "BLOCKED" for x in r4["scenarios"]), "verdict: --blocked-all did not make every scenario BLOCKED")
        check(r4.get("harness", {}).get("fingerprint") == "abc123", "verdict: the harness fingerprint is not in the result")
        # A mark may not say PASS or FAIL.
        write(u2, "marks.jsonl", [{"check": "stopped.c", "outcome": "PASS", "reason": "trust me"}])
        p3 = subprocess.run([sys.executable, os.path.join(ROOT, "lib", "verdict.py"), "--catalog", cp, "--record", u2,
                             "--ref-name", "fixture", "--ref-sha", "0" * 40, "--avenue", "goatbuilder", "--out", out + ".3"], capture_output=True, text=True)
        check(p3.returncode != 0 and "may say only" in p3.stderr + p3.stdout, "verdict: a PASS mark was accepted")
        try:
            import jsonschema
            jsonschema.validate(json.load(open(out)), json.load(open(os.path.join(ROOT, "result.schema.json"))))
        except ImportError:
            FAIL.append("jsonschema is not installed")


catalog_checks()
verdict_checks()
for f in FAIL:
    print(f"FAIL {f}")
print("ok: federation e2e harness checks" if not FAIL else f"{len(FAIL)} harness check(s) failed")
sys.exit(1 if FAIL else 0)
