#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
#
# test_runtime_health.py — the runtime-health gate (release-graduation.md §4)
# and the host reading behind it (ops/release/rung-health.py).
#
# The gate grades the record's `runtime_health.<lower rung>` block: captured
# no earlier than the soak's end, on the candidate digest, zero restarts, zero
# OOM kills, peak memory at or under the ceiling. Each refusal is asserted on
# its own, plus the recorded override and the C10 hold on the dev->preprod
# hop, which `graduate` never runs.
#
# Plain python3 + asserts, no pytest — the ops/release/tests convention.
# Exit codes: 0 all pass, 1 one or more failed.

import argparse
import datetime as dt
import importlib.util
import io
import os
import pathlib
import contextlib
import shutil
import sys
import tempfile

import yaml

HERE = os.path.dirname(os.path.abspath(__file__))
RELEASE = os.path.normpath(os.path.join(HERE, ".."))


def load(name, file):
    spec = importlib.util.spec_from_file_location(name, os.path.join(RELEASE, file))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


g = load("graduate", "graduate.py")
rh = load("rung_health", "rung-health.py")

DIGEST = "sha256:" + "a" * 64
REF = "ghcr.io/{{ ghcr_org }}/thing:sha-abc1234@" + DIGEST
HOP = {"from": "preprod", "to": "prod", "status": "live", "gates": [
    {"id": "candidate-record"}, {"id": "soak", "min_days": 2},
    {"id": "runtime-health", "max_memory_fraction": 0.8}]}
DEV_HOP = {"from": "dev", "to": "preprod", "status": "live", "gates": [
    {"id": "candidate-record"}, {"id": "runtime-health"}]}
TODAY = dt.date(2026, 8, 12)


def block(**over):
    b = {"captured": "2026-08-12T10:00:00Z", "deployed": "2026-08-10T09:00:00Z",
         "digests": [DIGEST], "restarts": 0, "oom_kills": 0, "peak_memory_fraction": 0.42}
    b.update(over)
    return b


def record(rung="preprod", **over):
    rec = {"component": "thing", "class": "ladder", "candidate_ref": REF,
           "opened": "2026-08-01", "state": "candidate",
           "evidence": {"validation": "rows PASS", "preprod_convergence": "console"},
           "runtime_health": {rung: block()}}
    rec.update(over)
    return rec


FAILURES = []


def check(name, cond, detail=""):
    print(f"  {'ok  ' if cond else 'FAIL'} {name}" + ("" if cond else f" {detail}"))
    if not cond:
        FAILURES.append(name)


def rt(rec, hop=HOP):
    return [f for f in g.gate_failures(rec, hop, TODAY) if f.startswith("runtime-health")]


print("gate")
check("a clean capture after the soak passes", rt(record()) == [], rt(record()))

r = record(); r["runtime_health"]["preprod"]["restarts"] = 8
f = rt(r)
check("a restart refuses", f == ["runtime-health: preprod restarted 8 time(s) since deploy"], f)

r = record(); r["runtime_health"]["preprod"]["oom_kills"] = 25
f = rt(r)
check("an OOM kill refuses", f == ["runtime-health: preprod OOM-killed 25 time(s) since deploy"], f)

r = record(); r["runtime_health"]["preprod"]["peak_memory_fraction"] = 0.93
f = rt(r)
check("peak memory over the ceiling refuses",
      f == ["runtime-health: preprod peak memory reached 0.93 of its limit, ceiling 0.8"], f)

r = record(); r["runtime_health"]["preprod"]["peak_memory_fraction"] = 0.8
check("peak memory AT the ceiling passes", rt(r) == [], rt(r))

r = record(); r["runtime_health"]["preprod"]["peak_memory_fraction"] = None
f = rt(r)
check("no memory limit refuses by default", len(f) == 1 and "no limit to grade against" in f[0], f)
lax = {**HOP, "gates": [{"id": "soak", "min_days": 2},
                        {"id": "runtime-health", "require_memory_limit": False}]}
check("... and passes when the gate says require_memory_limit: false", rt(r, lax) == [], rt(r, lax))

r = record(); r["runtime_health"]["preprod"]["oom_kills"] = None
f = rt(r)
check("an unreadable OOM count is unknown, never zero", f == [
    "runtime-health: preprod oom_kills unknown — the capture could not read it"], f)

f = rt(record(runtime_health={}))
check("a missing block refuses as awaiting", len(f) == 1 and f[0].startswith("runtime-health awaiting:")
      and "runtime_health.preprod" in f[0], f)

r = record(); r["runtime_health"]["preprod"]["captured"] = "2026-08-11T08:00:00Z"
f = rt(r)
check("a capture taken before the soak ended refuses", f == [
    "runtime-health awaiting: runtime_health.preprod was captured 23 h after deploy, "
    "needs >= 48 h (the soak)"], f)

r = record(); r["runtime_health"]["preprod"]["digests"] = ["sha256:" + "b" * 64]
f = rt(r)
check("a capture of a different digest refuses", f == [
    "runtime-health: runtime_health.preprod was not captured on the candidate digest aaaaaaaaaaaa"], f)

r = record(); r["runtime_health"]["dev"] = r["runtime_health"].pop("preprod")
check("the gate grades the hop's LOWER rung, not any rung",
      len(rt(r)) == 1 and rt(r)[0].startswith("runtime-health awaiting"), rt(r))

r = record(); r["runtime_health"]["preprod"]["restarts"] = 3
r["runtime_health_override"] = {"preprod": {"by": "A Person", "reason": "restarts were a host reboot"}}
check("a recorded override (by + reason) satisfies the gate", rt(r) == [], rt(r))
r["runtime_health_override"] = {"preprod": {"by": "", "reason": "x"}}
check("an override naming nobody does not", len(rt(r)) == 1, rt(r))

r = record(); r["runtime_health"]["preprod"]["captured"] = dt.datetime(2026, 8, 12, 10, 0, 0)
check("a capture time YAML already parsed reads the same", rt(r) == [], rt(r))

print("readiness")
rd = g.candidate_readiness(record(runtime_health={}, opened="2026-08-11"), HOP, TODAY)
check("an awaited capture during the soak reads soaking, not blocked", rd["state"] == "soaking", rd)
r = record(); r["runtime_health"]["preprod"]["oom_kills"] = 2
rd = g.candidate_readiness(r, HOP, TODAY)
check("a measured failure reads blocked", rd["state"] == "blocked", rd)
check("next step names runtime health",
      "runtime health" in g._next_step({"state": "blocked", "class": "ladder", "blockers": rd["blockers"]}))


# ---- graduate --override-runtime-health and check C10, on a scratch tree
def scratch_tree(rec, preprod_value):
    root = pathlib.Path(tempfile.mkdtemp())
    (root / "ops/release/records/thing").mkdir(parents=True)
    (root / "ops/pins").mkdir(parents=True)
    ladder = {"rungs": [{"name": "dev", "goatnet": "net_dev", "status": "live"},
                        {"name": "preprod", "goatnet": "net", "status": "live"},
                        {"name": "prod", "goatnet": "net_prod", "status": "live"}],
              "hops": [DEV_HOP, {**HOP, "gates": HOP["gates"] + [{"id": "operator-ack"}]}],
              "class_overrides": {}, "legacy_skew_allowlist": {}}
    (root / "ops/release/ladder.yaml").write_text(yaml.safe_dump(ladder))
    pins = {"goatnets": {"net_dev": {}, "net": {}, "net_prod": {}},
            "components": {"thing": {"fleet": "ghcr.io/{{ ghcr_org }}/thing:sha-0000000@sha256:" + "0" * 64,
                                     "deviations": {"net_dev": {"value": REF},
                                                    "net": {"value": preprod_value}}}}}
    (root / "ops/pins/fleet-pins.yaml").write_text(yaml.safe_dump(pins))
    path = root / "ops/release/records/thing/20260801-aaaaaaaaaaaa.yaml"
    path.write_text(yaml.safe_dump(rec, sort_keys=False))
    return root, path


def point(root):
    g.REPO = root
    g.LADDER = root / "ops/release/ladder.yaml"
    g.PINS = root / "ops/pins/fleet-pins.yaml"
    g.RECORDS = root / "ops/release/records"
    g.SETS = root / "ops/release/sets"


def run(fn, **kw):
    out, err = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        try:
            rc = fn(argparse.Namespace(**kw))
        except SystemExit as e:
            rc = e.code
    return rc, out.getvalue() + err.getvalue()


print("graduate")
saved = (g.REPO, g.LADDER, g.PINS, g.RECORDS, g.SETS)
bad = record(opened="2026-08-01")
bad["runtime_health"]["dev"] = block(captured="2026-08-03T10:00:00Z", deployed="2026-08-01T09:00:00Z")
bad["runtime_health"]["preprod"]["oom_kills"] = 4
root, path = scratch_tree(bad, REF)
point(root)
rc, text = run(g.cmd_graduate, component="thing", ack="A Person", override_runtime_health=None)
check("graduate refuses on a failing runtime-health gate", rc == 1 and "OOM-killed 4" in text, text)
rc, text = run(g.cmd_graduate, component="thing", ack=None, override_runtime_health="known leak")
check("an override without --ack is refused", rc == 1 and "needs --ack" in text, text)
rc, text = run(g.cmd_graduate, component="thing", ack="A Person",
               override_runtime_health="leak understood; limit raised in the next candidate")
saved_rec = yaml.safe_load(path.read_text())
ov = (saved_rec.get("runtime_health_override") or {}).get("preprod") or {}
check("graduate with an override succeeds", rc == 0, text)
check("... and writes who, why and what it overrode into the record",
      ov.get("by") == "A Person" and ov.get("reason", "").startswith("leak understood")
      and any("OOM-killed 4" in x for x in ov.get("failures", [])), ov)
shutil.rmtree(root)

print("check C10")
ok = record(opened="2026-08-01")
ok["runtime_health"]["dev"] = block(captured="2026-08-03T10:00:00Z", deployed="2026-08-01T09:00:00Z")
root, _ = scratch_tree(ok, REF)
point(root)
rc, text = run(g.cmd_check)
check("C10 passes a candidate on preprod whose dev capture is clean", rc == 0, text)
shutil.rmtree(root)

nodev = record(opened="2026-08-01")
root, _ = scratch_tree(nodev, REF)
point(root)
rc, text = run(g.cmd_check)
check("C10 refuses a candidate on preprod with no dev capture",
      rc == 1 and "C10:" in text and "runtime_health.dev" in text, text)
lad = yaml.safe_load((root / "ops/release/ladder.yaml").read_text())
lad["hops"][0]["gates"][1]["records_opened_from"] = "2026-08-05"
(root / "ops/release/ladder.yaml").write_text(yaml.safe_dump(lad))
rc, text = run(g.cmd_check)
check("C10 leaves a record opened before records_opened_from alone", rc == 0, text)
lad["hops"][0]["gates"][1]["max_memory_fraction"] = 1.5
(root / "ops/release/ladder.yaml").write_text(yaml.safe_dump(lad))
rc, text = run(g.cmd_check)
check("C1 refuses a ceiling outside (0, 1]", rc == 1 and "max_memory_fraction" in text, text)
shutil.rmtree(root)

ondev = record(opened="2026-08-01")
other = "ghcr.io/{{ ghcr_org }}/thing:sha-1111111@sha256:" + "1" * 64
root, _ = scratch_tree(ondev, other)
point(root)
rc, text = run(g.cmd_check)
check("C10 does not hold a candidate still on dev only", "C10:" not in text, text)
shutil.rmtree(root)

bad_ov = record(opened="2026-08-01")
bad_ov["runtime_health"]["dev"] = block(captured="2026-08-03T10:00:00Z", deployed="2026-08-01T09:00:00Z")
bad_ov["runtime_health_override"] = {"preprod": {"reason": "no name"}}
root, _ = scratch_tree(bad_ov, REF)
point(root)
rc, text = run(g.cmd_check)
check("C2 refuses an override with no by", rc == 1 and "runtime_health_override.preprod" in text, text)
shutil.rmtree(root)
g.REPO, g.LADDER, g.PINS, g.RECORDS, g.SETS = saved

print("rung-health")
probe = {"container": "thing", "id": "f" * 64, "created": "2026-08-10T09:00:00.123456789Z",
         "started": "2026-08-10T09:00:01.5Z", "running": True, "restart_count": 0,
         "oom_killed_flag": False, "memory_limit": 1000, "image": REF.replace("{{ ghcr_org }}", "org"),
         "repo_digests": [], "memory_peak": 420, "memory_current": 300, "oom_kills_kernel": 0}
row = rh.container_facts("host-a", probe)
check("a clean container folds to zero counts and its fraction",
      (row["restarts"], row["oom_kills"], row["peak_memory_fraction"], row["digests"]) == (0, 0, 0.42, [DIGEST]), row)
check("docker's nanosecond time reads as seconds UTC", row["created"] == "2026-08-10T09:00:00Z", row)
row2 = rh.container_facts("host-a", {**probe, "oom_kills_kernel": None, "kernel_log_error": "no journal",
                                      "restart_count": 2, "cgroup_oom_kill": 0})
check("no journal and a restart: the OOM count is unknown, not the reset cgroup counter",
      row2["oom_kills"] is None, row2)
row3 = rh.container_facts("host-a", {**probe, "oom_kills_kernel": None, "cgroup_oom_kill": 0})
check("no journal, no restart: the cgroup counter covers the window", row3["oom_kills"] == 0, row3)
row4 = rh.container_facts("host-a", {**probe, "oom_killed_flag": True})
check("State.OOMKilled counts when the journal shows none", row4["oom_kills"] == 1, row4)
row5 = rh.container_facts("host-a", {**probe, "memory_limit": 0, "cgroup_memory_max": None})
check("no limit: the fraction is unknown", row5["peak_memory_fraction"] is None, row5)
row6 = rh.container_facts("host-a", {**probe, "memory_peak": None})
check("no peak counter: current usage, named as such",
      row6["memory_peak_bytes"] == 300 and row6["memory_peak_source"].startswith("current usage"), row6)
now = dt.datetime(2026, 8, 12, 10, 0, 0, tzinfo=dt.timezone.utc)
blk = rh.summarise("thing", "net", "preprod", [row, {**row, "restarts": 1, "peak_memory_fraction": 0.5}], now)
check("the block sums restarts and takes the highest fraction",
      (blk["restarts"], blk["peak_memory_fraction"], blk["deployed"]) == (1, 0.5, "2026-08-10T09:00:00Z"), blk)
state, fails = rh.verdict(blk, "preprod", REF, (0.8, 2, True))
check("rung-health's verdict is the gate's", state == "fail" and fails == [
    "runtime-health: preprod restarted 1 time(s) since deploy"], (state, fails))
blk = rh.summarise("thing", "net", "preprod", [row, {"host": "host-a", "container": "x", "error": "no such container"}], now)
state, _ = rh.verdict(blk, "preprod", REF, (0.8, 2, True))
check("an unreadable container is unknown, and its counts are not zero",
      state == "unknown" and blk["restarts"] is None, blk)
state, fails = rh.verdict(rh.summarise("thing", "net", "preprod", [row], now), "preprod", REF, (0.8, 2, True))
check("a clean reading passes", state == "pass", fails)

print()
if FAILURES:
    print(f"{len(FAILURES)} runtime-health assertion(s) failed: {', '.join(FAILURES)}")
    sys.exit(1)
print("all runtime-health assertions hold")
