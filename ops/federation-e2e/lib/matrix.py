#!/usr/bin/env python3
"""matrix — one table of ref x scenario from every result file under a directory.

    matrix.py --out DIR [--baseline main]

Reads DIR/**/<sha12>-<avenue>.json (fed-e2e-result/1, written by `env run --out DIR`),
keeps the newest result per (ref name, avenue), and writes DIR/latest.md: one row per ref
and avenue, one column per scenario, cells in the outcome words. A cell that is worse than
the baseline's (main) for the same scenario and avenue is marked REGRESSION, and listed
below the table with the reason. Prints the regressions, one per line, and exits 0.
"""
import argparse
import glob
import json
import os

RANK = {"PASS": 0, "NOT APPLICABLE": 0, "SKIP": 1, "BLOCKED": 2, "FAIL": 3}
SHORT = {"PASS": "PASS", "FAIL": "FAIL", "BLOCKED": "BLOCKED", "SKIP": "SKIP", "NOT APPLICABLE": "N/A"}


def load(out):
    """Every result under out (archive*/ left out), merged per (ref name, avenue): for each scenario the
    newest result, with the harness revision it was measured on."""
    rows = {}
    for p in glob.glob(os.path.join(out, "**", "*-*.json"), recursive=True):
        rel = os.path.relpath(p, out)
        if rel.split(os.sep)[0].startswith("archive"):
            continue
        try:
            with open(p, encoding="utf-8") as fh:
                r = json.load(fh)
        except (OSError, ValueError):
            continue
        if r.get("schema") != "fed-e2e-result/1":
            continue
        h = r.get("harness", {})
        rev = h.get("revision") or h.get("fingerprint") or "unknown"
        name = r["ref"]["name"]
        if r.get("directory"):
            name += f" (directory saltlick {r['directory']['sha'][:12]})"
        k = (name, r["avenue"])
        row = rows.setdefault(k, {"ref": r["ref"], "avenue": r["avenue"], "scen": {}, "paths": set(), "finished": ""})
        if r["finished"] > row["finished"]:
            row["finished"], row["ref"] = r["finished"], r["ref"]
        for sc in r["scenarios"]:
            cur = row["scen"].get(sc["id"])
            if cur is None or r["finished"] > cur["_finished"]:
                row["scen"][sc["id"]] = dict(sc, _finished=r["finished"], _rev=rev, _path=rel)
        row["paths"].add(rel)
    for row in rows.values():
        outs = [s["outcome"] for s in row["scen"].values() if s.get("required", True)]
        row["verdict"] = next((w for w in ("FAIL", "BLOCKED", "SKIP") if w in outs), "PASS")
    return rows


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", required=True)
    ap.add_argument("--baseline", default="main")
    a = ap.parse_args()
    best = load(a.out)
    scen = []
    for r in best.values():
        for sid in r["scen"]:
            if sid not in scen:
                scen.append(sid)
    order = sorted(best, key=lambda k: (k[0] != a.baseline, k[0] != "integration", k[0], k[1]))
    lines = ["# Federation e2e: every ref against every scenario", "",
             "Outcome words: PASS (expected evidence observed), FAIL (violation observed), BLOCKED "
             "(prerequisite or evidence unavailable), SKIP (not exercised), N/A = NOT APPLICABLE "
             "(excluded by the agreement). A required BLOCKED or SKIP never counts as passing. "
             "A row merges every scenario subset measured for that ref and avenue, newest result per scenario. "
             f"A cell is compared with `{a.baseline}` only when both were measured on the same harness revision; "
             "REGRESSION marks a cell worse than a PASS on main; `≠` marks a cell measured on another revision than main's.", "",
             "Columns: " + ", ".join(f"{i + 1} `{s}`" for i, s in enumerate(scen)), "",
             "| ref | sha | avenue | verdict | " + " | ".join(str(i + 1) for i in range(len(scen))) + " |",
             "|---|---|---|---|" + "---|" * len(scen)]
    regress, notcmp = [], []
    for k in order:
        r = best[k]
        base = best.get((a.baseline, k[1]))
        bby = base["scen"] if base else {}
        cells = []
        for sid in scen:
            s = r["scen"].get(sid)
            if not s:
                cells.append("·")
                continue
            c = SHORT[s["outcome"]]
            b = bby.get(sid)
            if k[0] != a.baseline and b is not None:
                if b["_rev"] != s["_rev"]:
                    c += " ≠"
                    notcmp.append(f"{k[0]}@{r['ref']['sha'][:12]} {k[1]} {sid}: revision {s['_rev']}, {a.baseline} {b['_rev']}")
                elif RANK[s["outcome"]] > RANK[b["outcome"]] and b["outcome"] == "PASS":
                    c = f"**{c} REGRESSION**"
                    regress.append(f"{k[0]}@{r['ref']['sha'][:12]} {k[1]} {sid}: {s['outcome']} (main PASS) — {s['reason'][:200]}")
            cells.append(c)
        lines.append(f"| {k[0]} | `{r['ref']['sha'][:12]}` | {k[1]} | {r['verdict']} | " + " | ".join(cells) + " |")
    lines += ["", "## Regressions against " + a.baseline, ""]
    lines += [f"- {x}" for x in regress] or ["None."]
    lines += ["", f"## Not comparable with {a.baseline} (measured with a different harness)", ""]
    lines += [f"- {x}" for x in notcmp] or ["None."]
    # Scenario module-revert: each module reverted on the integration head against the
    # integration head before that module (lib/watch.sh --reverts).
    lines += ["", "## Module reverts on the integration branch", "",
              "Each module reverted on the integration head, against the integration head before it. "
              + "PASS: every scenario has the same outcome on both; FAIL: the differences are listed.", ""]
    rows = 0
    for (name, av), r in sorted(best.items()):
        if not name.startswith("integration-revert-"):
            continue
        m = name[len("integration-revert-"):]
        before = best.get((f"integration-before-{m}", av))
        if not before:
            lines.append(f"- module `{m}` {av}: BLOCKED — the integration head before it is not measured yet")
            rows += 1
            continue
        bb = {sid: x["outcome"] for sid, x in before["scen"].items()}
        diffs = [f"{sid} {bb.get(sid)} -> {x['outcome']}" for sid, x in r["scen"].items() if sid in bb and bb[sid] != x["outcome"]]
        lines.append(f"- module `{m}` {av}: " + ("PASS" if not diffs else "FAIL — " + "; ".join(diffs)))
        rows += 1
    if not rows:
        lines.append("None measured yet.")
    with open(os.path.join(a.out, "latest.md"), "w", encoding="utf-8") as fh:
        fh.write("\n".join(lines) + "\n")
    for x in regress:
        print(x)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
