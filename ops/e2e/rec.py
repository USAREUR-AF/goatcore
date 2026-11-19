#!/usr/bin/env python3
"""rec — run one step of an e2e run and record it (docs/design/deployment-e2e.md §The run record).

    rec.py <name> [--expect-exit N] [--expect-in TEXT]... [--expect-not-in TEXT]...
                  [--xfail REASON] [--cwd DIR] [--note TEXT] -- <command…>

Appends one JSON object per step to $E2E_RECORD_DIR/report.jsonl (schema:
ops/e2e/report.schema.json), writes the step's full output to
$E2E_RECORD_DIR/<nn>-<name>.log, prints one line, and exits 0 when the step
passed (or failed as expected), 1 otherwise. A step passes when its exit code is
the expected one and every --expect-in text appears (and no --expect-not-in text
does) in its combined output.
"""
import argparse
import json
import os
import subprocess
import sys
import time



def read_text(path):
    with open(path, encoding="utf-8") as fh:
        return fh.read()

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("name")
    ap.add_argument("--expect-exit", type=int, default=0)
    ap.add_argument("--expect-in", action="append", default=[])
    ap.add_argument("--expect-not-in", action="append", default=[])
    ap.add_argument("--xfail", default="")
    ap.add_argument("--cwd", default=None)
    ap.add_argument("--note", default="")
    ap.add_argument("--env", action="append", default=[], help="KEY=VALUE for this step only")
    argv = sys.argv[1:]
    if "--" not in argv:
        ap.error("no command after --")
    i = argv.index("--")
    a = ap.parse_args(argv[:i])
    cmd = argv[i + 1:]
    if not cmd:
        ap.error("no command after --")
    rdir = os.environ.get("E2E_RECORD_DIR") or os.path.join(os.getcwd(), "e2e-report")
    os.makedirs(rdir, exist_ok=True)
    report = os.path.join(rdir, "report.jsonl")
    n = sum(1 for _ in read_text(report).splitlines(True)) + 1 if os.path.exists(report) else 1
    env = dict(os.environ)
    for kv in a.env:
        k, _, v = kv.partition("=")
        env[k] = v
    started = time.time()
    # A command that cannot start (not found, not executable, a bad --cwd) is a failed step,
    # recorded like any other, never a crash that leaves no row: a lost step can hide a check.
    unstartable = ""
    try:
        p = subprocess.run(cmd, cwd=a.cwd, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        rc, text = p.returncode, p.stdout or ""
    except OSError as e:
        unstartable = f"the command could not start: {cmd[0]}: {e.strerror or e}"
        rc, text = 127, unstartable + "\n"
    dur = round(time.time() - started, 2)
    log = os.path.join(rdir, f"{n:02d}-{a.name}.log")
    with open(log, "w", encoding="utf-8") as fh:
        fh.write(text)
    missing = [t for t in a.expect_in if t not in text]
    present = [t for t in a.expect_not_in if t in text]
    ok = not unstartable and (rc == a.expect_exit) and not missing and not present
    if ok:
        status = "pass"
    elif a.xfail and not unstartable:
        status = "xfail"
    else:
        status = "fail"
    why = ""
    if not ok:
        parts = []
        if unstartable:
            parts.append(unstartable)
        elif rc != a.expect_exit:
            parts.append(f"exit {rc}, expected {a.expect_exit}")
        for t in missing:
            parts.append(f"missing text: {t!r}")
        for t in present:
            parts.append(f"unexpected text: {t!r}")
        why = "; ".join(parts)
    row = {
        "n": n, "name": a.name, "status": status, "command": cmd, "cwd": a.cwd or os.getcwd(),
        "exit": rc, "expected_exit": a.expect_exit, "duration_s": dur,
        "started": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(started)),
        "log": os.path.basename(log), "tail": text[-1500:], "why": why, "xfail": a.xfail, "note": a.note,
    }
    with open(report, "a", encoding="utf-8") as fh:
        fh.write(json.dumps(row) + "\n")
    mark = {"pass": "ok  ", "xfail": "XFAIL", "fail": "FAIL"}[status]
    print(f"{mark:5} {n:02d} {a.name} ({dur}s){' — ' + why if why else ''}{' [expected: ' + a.xfail + ']' if status == 'xfail' else ''}")
    sys.exit(0 if status in ("pass", "xfail") else 1)


if __name__ == "__main__":
    main()
