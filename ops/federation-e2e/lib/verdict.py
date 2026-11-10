#!/usr/bin/env python3
"""verdict — turn the recorded steps of one matrix run into scenario outcomes.

    verdict.py --catalog scenarios/catalog.yaml --record DIR [--record DIR]...
               --ref-name NAME --ref-sha SHA --avenue script|goatbuilder
               [--harness-sha SHA] [--only ID,ID] [--out results.json] [--markdown OUT]

Each record directory holds the rec.py rows (report.jsonl) of one unit, and optionally
marks.jsonl: rows {"check": "<scenario>.<check>", "outcome": "BLOCKED|SKIP|NOT APPLICABLE",
"reason": "..."} that a unit writes when it decides a check cannot run or is excluded by
the agreement. A unit that stopped writes stopped.json {"reason": "..."}.

A mark may only say BLOCKED, SKIP or NOT APPLICABLE: PASS and FAIL come from recorded
steps alone, never from a unit's say-so.

A check reads one recorded step (its `step`, with {A}, {B} and {n} replaced):
  a failed step -> FAIL, whatever else is recorded: a violation observed is never hidden;
  a mark -> the mark's outcome and reason, even beside a passing step: no PASS where the
    unit itself declared the check blocked, skipped or excluded;
  a passing step -> PASS;
  no step, the unit stopped -> BLOCKED (the unit stopped before it);
  no step, the unit never ran -> SKIP.
A scenario is FAIL if a required check FAILed, else BLOCKED if one is BLOCKED, else SKIP
if one is SKIP, else NOT APPLICABLE if all are, else PASS. A required BLOCKED or SKIP is
never a pass: the run exits 0 only when every required scenario is PASS or NOT APPLICABLE.

Outcome words are exact: PASS, FAIL, BLOCKED, SKIP, NOT APPLICABLE.
"""
import argparse
import datetime as dt
import json
import os
import re
import sys

import yaml

HERE = os.path.dirname(os.path.abspath(__file__))
WORDS = ("PASS", "FAIL", "BLOCKED", "SKIP", "NOT APPLICABLE")
MARK_WORDS = ("BLOCKED", "SKIP", "NOT APPLICABLE")


def load_jsonl(path):
    if not os.path.exists(path):
        return []
    with open(path, encoding="utf-8") as fh:
        return [json.loads(line) for line in fh if line.strip()]


def scenario_outcome(checks):
    req = [c for c in checks if c["required"]] or checks
    outs = [c["outcome"] for c in req]
    for w in ("FAIL", "BLOCKED", "SKIP"):
        if w in outs:
            first = next(c for c in req if c["outcome"] == w)
            return w, f"{first['id']}: {first['reason']}"
    if outs and all(o == "NOT APPLICABLE" for o in outs):
        return "NOT APPLICABLE", req[0]["reason"]
    if not outs:
        return "SKIP", "the scenario declares no checks"
    return "PASS", ""


def judge(catalog, records, avenue, only):
    steps, marks, units = {}, {}, {}
    for rdir in records:
        unit = os.path.basename(os.path.normpath(rdir))
        stopped = None
        sp = os.path.join(rdir, "stopped.json")
        if os.path.exists(sp):
            with open(sp, encoding="utf-8") as fh:
                stopped = json.load(fh).get("reason") or "the unit stopped"
        units[unit] = stopped
        for row in load_jsonl(os.path.join(rdir, "report.jsonl")):
            row["_unit"], row["_dir"] = unit, rdir
            steps[(unit, row["name"])] = row
        for m in load_jsonl(os.path.join(rdir, "marks.jsonl")):
            if m.get("outcome") not in MARK_WORDS:
                raise SystemExit(f"verdict: {rdir}/marks.jsonl: a mark may say only BLOCKED, SKIP or NOT APPLICABLE, not {m.get('outcome')!r}")
            if not (m.get("reason") or "").strip():
                raise SystemExit(f"verdict: {rdir}/marks.jsonl: the mark on {m.get('check')} has no reason")
            marks[(unit, m["check"])] = m
    out = []
    params = catalog.get("params", {})
    for sc in catalog["scenarios"]:
        if only and sc["id"] not in only:
            continue
        unit = (sc.get("unit") or {}).get(avenue) if isinstance(sc.get("unit"), dict) else sc.get("unit")
        checks = []
        for ck in sc.get("checks", []):
            cid = f"{sc['id']}.{ck['id']}"
            subst = dict(params)
            subst.update((catalog.get("avenue_params") or {}).get(avenue, {}))
            subst.update(((sc.get("params") or {}).get(avenue) or {}) if isinstance(sc.get("params"), dict) else {})
            step = ck.get("step", "")
            for k, v in subst.items():
                step = step.replace("{" + k + "}", str(v))
            c = {"id": ck["id"], "required": bool(ck.get("required", True)), "observer": "local"}
            if step:
                c["step"] = step
            row = None
            if unit and step:
                if any(ch in step for ch in "*[]()|+?") and ck.get("regex"):
                    pat = re.compile(step)
                    hits = [r for (u, n), r in steps.items() if u == unit and pat.fullmatch(n)]
                    row = hits[-1] if hits else None
                else:
                    row = steps.get((unit, step))
            mark = marks.get((unit, cid)) or marks.get((unit, ck["id"]))
            if row is not None and row["status"] != "pass":
                c["outcome"], c["reason"] = "FAIL", (row.get("why") or "failed")
                c["evidence"] = os.path.join(row["_unit"], row["log"])
                c["duration_s"] = row.get("duration_s", 0)
            elif mark is not None:
                c["outcome"], c["reason"] = mark["outcome"], mark["reason"]
                if row is not None:
                    c["reason"] += f" (step {step} also recorded and passed; the mark decides)"
                    c["evidence"] = os.path.join(row["_unit"], row["log"])
            elif row is not None:
                c["outcome"], c["reason"] = "PASS", ""
                c["evidence"] = os.path.join(row["_unit"], row["log"])
                c["duration_s"] = row.get("duration_s", 0)
            elif sc.get("not_built"):
                c["outcome"], c["reason"] = "SKIP", f"not built: {sc['not_built']}"
            elif not unit:
                c["outcome"], c["reason"] = "SKIP", f"no unit runs this scenario for avenue {avenue}"
            elif unit not in units:
                c["outcome"], c["reason"] = "SKIP", f"unit {unit} was not run"
            elif units[unit]:
                c["outcome"], c["reason"] = "BLOCKED", f"unit {unit} stopped before this step: {units[unit]}"
            else:
                c["outcome"], c["reason"] = "BLOCKED", f"unit {unit} finished without recording step {step or '(none)'}"
            checks.append(c)
        o, why = scenario_outcome(checks)
        s = {
            "id": sc["id"], "title": sc["title"], "pass_condition": sc["pass_condition"].strip(),
            "runs_where": sc.get("runs_where", "local"), "required": bool(sc.get("required", True)),
            "outcome": o, "reason": why, "checks": checks,
        }
        if sc.get("known_failure"):
            s["known_failure"] = sc["known_failure"]
        out.append(s)
    return out


def verdict_of(scenarios):
    counts = {w: 0 for w in WORDS}
    for s in scenarios:
        counts[s["outcome"]] += 1
    req = [s["outcome"] for s in scenarios if s["required"]]
    for w in ("FAIL", "BLOCKED", "SKIP"):
        if w in req:
            return {"outcome": w, "counts": counts, "exit": 1}
    return {"outcome": "PASS", "counts": counts, "exit": 0}


def markdown(res):
    v = res["verdict"]
    lines = [f"## Federation e2e: {res['ref']['name']} @ {res['ref']['sha'][:12]}, avenue {res['avenue']}: {v['outcome']}", "",
             " · ".join(f"{k} {n}" for k, n in v["counts"].items()), ""]
    if res.get("directory"):
        lines += [f"Peering directory: saltlick {res['directory']['ref']} @ {res['directory']['sha'][:12]}, not the pinned image.", ""]
    lines += ["| scenario | outcome | where | reason |", "|---|---|---|---|"]
    for s in res["scenarios"]:
        lines.append(f"| `{s['id']}` | {s['outcome']} | {s['runs_where']} | {s['reason'].replace('|', '/')[:300]} |")
    lines += ["", "### Checks", ""]
    for s in res["scenarios"]:
        lines.append(f"**{s['id']}** — {s['title']}. Pass condition: {s['pass_condition']}")
        lines.append("")
        for c in s["checks"]:
            req = "required" if c["required"] else "optional"
            lines.append(f"- {c['outcome']} `{c['id']}` ({req}){': ' + c['reason'] if c['reason'] else ''}")
        lines.append("")
    return "\n".join(lines) + "\n"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--catalog", default=os.path.join(HERE, "..", "scenarios", "catalog.yaml"))
    ap.add_argument("--record", action="append", default=[])
    ap.add_argument("--ref-name", required=True)
    ap.add_argument("--ref-sha", required=True)
    ap.add_argument("--merged", action="append", default=[])
    ap.add_argument("--avenue", required=True, choices=("script", "goatbuilder"))
    ap.add_argument("--harness-sha", default="")
    ap.add_argument("--harness-fp", default="", help="the harness fingerprint (env fingerprint)")
    ap.add_argument("--blocked-all", default="", help="every check BLOCKED with this reason (the ref did not build)")
    ap.add_argument("--run-id", default="")
    ap.add_argument("--started", default="")
    ap.add_argument("--only", default="")
    ap.add_argument("--out")
    ap.add_argument("--markdown")
    ap.add_argument("--versions", default="", help="JSON object: sovereign id -> tool version")
    ap.add_argument("--directory-ref", default="", help="the saltlick ref the peering directory was built from (env --directory-ref)")
    ap.add_argument("--directory-sha", default="", help="that ref's commit")
    a = ap.parse_args()
    with open(a.catalog, encoding="utf-8") as fh:
        catalog = yaml.safe_load(fh)
    only = set(x for x in a.only.split(",") if x)
    now = dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    scen = judge(catalog, [] if a.blocked_all else a.record, a.avenue, only)
    if a.blocked_all:
        for sc in scen:
            for c in sc["checks"]:
                c["outcome"], c["reason"] = "BLOCKED", a.blocked_all
                c.pop("evidence", None)
            sc["outcome"], sc["reason"] = "BLOCKED", a.blocked_all
    ref = {"name": a.ref_name, "sha": a.ref_sha}
    if a.merged:
        ref["merged"] = a.merged
    res = {
        "schema": "fed-e2e-result/1",
        "run_id": a.run_id or f"{a.ref_sha[:12]}-{a.avenue}-{now}",
        "ref": ref, "avenue": a.avenue,
        "harness": dict({"sha": a.harness_sha or "unknown"}, **({"fingerprint": a.harness_fp} if a.harness_fp else {})),
        "started": a.started or now, "finished": now,
        "scenarios": scen, "verdict": verdict_of(scen),
    }
    if a.versions:
        res["versions"] = json.loads(a.versions)
    if a.directory_ref:
        res["directory"] = {"ref": a.directory_ref, "sha": a.directory_sha}
    # An unvalidated result is not written: jsonschema is required.
    try:
        import jsonschema
    except ImportError:
        raise SystemExit("verdict: python3 jsonschema is required to validate the result; install it (pip install jsonschema)")
    with open(os.path.join(HERE, "..", "result.schema.json"), encoding="utf-8") as fh:
        jsonschema.validate(res, json.load(fh))
    text = json.dumps(res, indent=2) + "\n"
    if a.out:
        with open(a.out, "w", encoding="utf-8") as fh:
            fh.write(text)
    md = markdown(res)
    if a.markdown:
        with open(a.markdown, "w", encoding="utf-8") as fh:
            fh.write(md)
    if not a.out and not a.markdown:
        sys.stdout.write(md)
    return res["verdict"]["exit"]


if __name__ == "__main__":
    sys.exit(main())
