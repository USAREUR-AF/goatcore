#!/usr/bin/env python3
"""graduate.py — guide and observe release graduation across the goatnet ladder.

Canon: docs/design/release-graduation.md (ADR 1120). Reads the ladder
(ops/release/ladder.yaml), the pin table (ops/pins/fleet-pins.yaml) and the
promotion records (ops/release/records/). It never rewrites fleet-pins.yaml —
that file's comments are load-bearing review context, so pin edits are made by
the human/PR that owns them and this tool PRINTS the exact edit, then `check`
refuses trees where records and pins disagree.

Subcommands:
  status                       cross-rung matrix: per component, the effective
                               value on each live rung, graded.
  open <comp> --ref R --reason T   start a candidate: writes the record, prints
                               the preprod deviation edit.
  graduate <comp> --ack NAME   verify the hop gates, mark the record graduated,
                               print the fold-to-fleet edit.
  hotfix <comp> --ref R --reason T --ack NAME   expedited prod path: record +
                               prod deviation edit; owes a backfill (status
                               shows it until the ref graduates normally).
  review [--ack NAME] [--json] the start-of-day / end-of-day review: every open
                               record sorted by what a person must DO — ACT NOW
                               (all gates pass), SOAKING (runnable date, evidence
                               to fill first), BLOCKED (what and by when), and
                               the tree's own check verdict — each with the exact
                               command. Read-only; graduation is never automated
                               (ADR 1177).
  readiness [--out F]          the `release-readiness-v1` document: everything
                               `status` grades plus, per component, the open
                               candidate's soak clock (and any recorded
                               waiver), the hop gates it still fails (the SAME
                               gate_failures `graduate` runs), the ADR 1060
                               ledger join, the pinned build's commit age and
                               how many commits since it touched the paths the
                               component's image workflow builds on. Read-only,
                               derived, no verdict of its own; the consistency
                               console renders it.
  check                        CI invariants (see CHECKS below). Exit 0/1.

CHECKS (fail-closed, current-tree — not diff-scoped, so a violation cannot
hide behind an unrelated PR):
  C1  ladder.yaml parses, rung goatnets exist in fleet-pins, hop gates known.
  C2  every record parses, references a real component, digest-pinned ref.
  C3  preprod-ahead skew (preprod effective != prod effective) has an OPEN
      candidate record for that exact ref, or a legacy_skew_allowlist row.
  C4  prod-ahead skew has a hotfix record for that exact ref (backfill owed).
  C5  a graduated record's ref matches the current fleet value, unless a newer
      record for the component supersedes it.
  C6  no record is open for a ref that is no longer present on its rung
      (stale candidate — abandon or refresh it).
  C7  every graduation set parses, names known components, digest-pinned refs,
      and one existing record per member; no component sits in two OPEN sets.
  C8  an abandoned record opened on/after ABANDON_REASON_CUTOFF carries a
      machine-readable `abandoned_reason:` (a YAML comment does not count —
      nothing reading records as data can see one). Older records are
      grandfathered rather than back-filled with invented reasons.
  C9  an open hotfix record older than HOTFIX_BACKFILL_MAX_DAYS carries
      validation evidence — a fastlane's backfill is owed, not optional.
  C10 an open ladder candidate already running on the upper rung of a live
      hop that carries the runtime-health gate passes that gate for the lower
      rung (or carries a recorded override) — the dev->preprod move is a pin
      edit, not a `graduate`, so CI is where that hop is held.

GATE runtime-health (any hop): the record's `runtime_health.<lower rung>`
block, written by ops/release/rung-health.py at the end of the soak, shows the
candidate digest running with zero restarts, zero OOM kills and peak memory at
or under the gate's `max_memory_fraction` of the container limit. Refused
unless `runtime_health_override.<lower rung>` names who and why
(`graduate --override-runtime-health REASON --ack NAME` writes it).
"""

import argparse
import datetime as dt
import json
import pathlib
import re
import subprocess
import sys

import yaml

REPO = pathlib.Path(__file__).resolve().parents[2]
LADDER = REPO / "ops/release/ladder.yaml"
PINS = REPO / "ops/pins/fleet-pins.yaml"
RECORDS = REPO / "ops/release/records"
SETS = REPO / "ops/release/sets"

DIGEST_RE = re.compile(r"@sha256:[0-9a-f]{64}")
KNOWN_GATES = {"candidate-record", "soak", "convergence-evidence",
               "validation-evidence", "operator-ack", "runtime-health"}
# runtime-health: the ceiling on peak memory as a fraction of the container's
# limit when the gate entry in ladder.yaml does not set max_memory_fraction.
RUNTIME_HEALTH_MAX_MEMORY_FRACTION = 0.8
RECORD_STATES = {"candidate", "graduated", "abandoned"}
# C9: a fastlane's hotfix record may sit with empty validation evidence for
# this many days before `check` refuses the tree. One soak window plus room
# for the operator to run the rows on the next working day.
HOTFIX_BACKFILL_MAX_DAYS = 3
# C9 applies to hotfix records opened from this date on. The three fastlane
# records already open when the check landed (microscope netvis
# herd) still grade hotfix-backfill-owed in `status`;
# they are not failed retroactively for a rule that did not exist when they
# were written — the same posture as ABANDON_REASON_CUTOFF for C8.
HOTFIX_BACKFILL_CUTOFF = dt.date(2026, 9, 9)

# C8 applies to records opened from this date on. Everything older predates
# the field and is deliberately grandfathered — see cmd_check.
ABANDON_REASON_CUTOFF = "2026-08-13"
SET_STATES = {"open", "graduated", "abandoned"}
CLASSES = {"ladder", "env-local", "exempt"}


def die(msg):
    print(f"graduate: {msg}", file=sys.stderr)
    sys.exit(1)


def load_yaml(path):
    with open(path) as f:
        return yaml.safe_load(f)


def load_ladder():
    ladder = load_yaml(LADDER)
    rungs = ladder.get("rungs") or []
    live = [r for r in rungs if r.get("status") == "live"]
    if not live:
        die("ladder has no live rungs")
    return ladder, rungs, live


def load_pins():
    return load_yaml(PINS)


def effective(comp_def, goatnet):
    """Resolution for a goatnet: goatnet deviation -> fleet. Member-level
    deviations are intra-goatnet detail (node parity's job, not the ladder's).
    Returns (state, value): state in {value, absent, unresolved}."""
    dev = (comp_def.get("deviations") or {}).get(goatnet)
    if dev:
        if dev.get("absent"):
            return "absent", None
        if "value" in dev:
            return "value", dev["value"]
    fleet = comp_def.get("fleet")
    if fleet is None:
        return "unresolved", None
    return "value", fleet


def load_records():
    out = []
    if not RECORDS.is_dir():
        return out
    for p in sorted(RECORDS.glob("*/*.yaml")):
        try:
            rec = load_yaml(p)
        except yaml.YAMLError as e:
            out.append({"_path": p, "_error": str(e)})
            continue
        rec["_path"] = p
        out.append(rec)
    return out


def latest_record(records, comp, state=None, ref=None, cls=None):
    hits = [r for r in records
            if r.get("component") == comp and "_error" not in r
            and (state is None or r.get("state") == state)
            and (ref is None or r.get("candidate_ref") == ref)
            and (cls is None or (r.get("class") or "ladder") == cls)]
    return hits[-1] if hits else None


# ---------------------------------------------------------------------------
# Graduation sets (ADR 1125) — the unit of MOVEMENT, beside the per-component
# record which stays the unit of EVIDENCE.
#
# WHY THIS EXISTS. The promotion unit was one component's pin, so nothing in
# the model could express "these refs move together, or not at all". On
# a fix merged directly on top of another session's release
# candidate; the pin bump that carried it to prod also graduated the earlier
# session's console train ahead of its stated intent. Two sessions coordinated
# in prose and got the right outcome — the structural cause was untouched. A
# promotion unit smaller than the validated set produces that surprise
# structurally, not occasionally (Kargo's Freight, in this repo's terms).
#
# OPT-IN. A component in no set graduates exactly as before. Sets are for
# changes validated together; they are not a tax on every bump.
# ---------------------------------------------------------------------------


def load_sets():
    """Every committed graduation set, parse errors carried rather than raised
    (C7 reports them with a path a reviewer can open)."""
    out = []
    if not SETS.is_dir():
        return out
    for p in sorted(SETS.glob("*.yaml")):
        try:
            doc = load_yaml(p)
        except yaml.YAMLError as e:
            out.append({"_path": p, "_error": str(e)})
            continue
        doc["_path"] = p
        out.append(doc)
    return out


def set_members(doc):
    return [m for m in (doc.get("members") or []) if isinstance(m, dict)]


def open_set_for(sets, component):
    """The OPEN set this component belongs to, or None. C7 guarantees at most
    one, so returning the first is not a silent pick."""
    for doc in sets:
        if "_error" in doc or doc.get("state") != "open":
            continue
        if any(m.get("component") == component for m in set_members(doc)):
            return doc
    return None


def set_blockers(doc, records, hop, today):
    """Per-member gate failures across the whole set, member-labelled. A set is
    ready when EVERY member is ready; reporting only the first would make the
    second failure a surprise after the first is fixed."""
    out = []
    for m in set_members(doc):
        comp, ref = m.get("component"), m.get("ref")
        rec = latest_record(records, comp, state="candidate", ref=ref)
        if rec is None:
            out.append(f"{comp}: no OPEN candidate record for the set's exact ref")
            continue
        for f in gate_failures(rec, hop, today):
            out.append(f"{comp}: {f}")
    return out


# The component's grade is its worst hop. Ordered most to least serious.
RC_TAG_RE = re.compile(r":v\d+\.\d+\.\d+-rc\.\d+@")
GRADE_ORDER = ("UNDECLARED-SKEW", "hotfix-backfill-owed", "candidate", "legacy-declared", "aligned")


def grade_pair(name, lo_ref, hi_ref, records, legacy, top):
    """Grade one adjacent hop of the ladder, lower rung first: the rung a
    candidate reaches earlier (dev before preprod, preprod before prod) against
    the one above it. The lower rung running ahead is a candidate on its way
    (C3). The upper rung running ahead is a hotfix owing its backfill (C4) on
    the hop into prod (`top`); on a lower hop it is whatever its record says
    it is — a ladder candidate that reached preprod without a dev deviation is
    a candidate, not a fastlane. `legacy` is true only on the hop a
    legacy_skew_allowlist row excuses. An open hotfix record owes its backfill
    whichever rung of the hop runs its ref (§2)."""
    if lo_ref == hi_ref:
        # A fastlane (ADR 1127) moves EVERY rung to the new ref at once, so the
        # rungs agree and the component read `aligned` while its hotfix record
        # sat open with empty evidence — the debt was invisible in the one
        # place the design said it would show (onboard-runner and
        # id-admin-console both read aligned the morning after their
        # fastlanes). An open hotfix record for the ref the rungs agree on IS
        # the backfill owed, whatever the rungs say.
        rec = latest_record(records, name, state="candidate", ref=hi_ref)
        if rec and rec.get("class") == "hotfix":
            return "hotfix-backfill-owed"
        return "aligned"
    if legacy:
        return "legacy-declared"
    rec = latest_record(records, name, state="candidate", ref=lo_ref)
    if rec:
        if rec.get("class") == "hotfix":
            return "hotfix-backfill-owed"
        return "candidate"
    rec = latest_record(records, name, state="candidate", ref=hi_ref)
    if rec:
        if top or rec.get("class") == "hotfix":
            return "hotfix-backfill-owed"
        return "candidate"
    # A release-candidate tag on the lower rung of a lower hop is a candidate on
    # its way by construction (ADR 1173): rc.sh puts `<image>:vX.Y.Z-rc.N` on the
    # dev rung and opens the record only at preprod. Without this, a component
    # with no open record for the rung above refused its own first dev step (C3);
    # it passed in this repository only because each component's preprod ref
    # happened to carry an open record already (found by the hermetic e2e tier,
    # whose canary has no history).
    if not top and RC_TAG_RE.search(str(lo_ref or "")):
        return "candidate"
    return "UNDECLARED-SKEW"


def grade_component(name, comp_def, live_rungs, records, overrides, allow):
    cls = (overrides.get(name) or {}).get("class", "ladder")
    if cls == "exempt":
        return {"component": name, "class": cls, "grade": "exempt", "rungs": {}, "pairs": {}}
    per_rung = {}
    for r in live_rungs:
        st, val = effective(comp_def, r["goatnet"])
        per_rung[r["name"]] = {"state": st, "value": val}
    vals = {n: v for n, v in per_rung.items() if v["state"] == "value"}
    if len(vals) < 2:
        grade = "env-local" if any(v["state"] == "absent" for v in per_rung.values()) else "single-rung"
        return {"component": name, "class": cls, "grade": grade, "rungs": per_rung, "pairs": {}}
    # Every adjacent pair of rungs that resolve to a value, in ladder order.
    # Grading only the first and last rung read dev against prod once the dev
    # rung went live, so a deviation declared on the middle rung
    # alone was never inspected: a preprod-only candidate graded `aligned`
    # (GH #1303). The hops are where skew is defined (§6 C3/C4), so the hops
    # are what get graded, and the component carries its worst hop.
    names = list(vals)
    # The ladder's top hop (preprod->prod), read from the ladder itself, never
    # from whichever rungs resolve: a component prod does not run has no hop
    # into prod. The legacy allowlist predates the dev rung, so the top hop is
    # the only one it excuses.
    top_hop = (live_rungs[-2]["name"], live_rungs[-1]["name"])
    pairs = {}
    for lo, hi in zip(names, names[1:]):
        pairs[f"{lo}->{hi}"] = grade_pair(name, vals[lo]["value"], vals[hi]["value"], records,
                                          legacy=name in allow and (lo, hi) == top_hop,
                                          top=hi == top_hop[1])
    grade = next(g for g in GRADE_ORDER if g in pairs.values())
    return {"component": name, "class": cls, "grade": grade, "rungs": per_rung, "pairs": pairs}


def build_status():
    ladder, rungs, live = load_ladder()
    pins = load_pins()
    records = load_records()
    overrides = ladder.get("class_overrides") or {}
    allow = ladder.get("legacy_skew_allowlist") or {}
    rows = []
    for name, comp_def in (pins.get("components") or {}).items():
        rows.append(grade_component(name, comp_def, live, records, overrides, allow))
    return ladder, live, records, rows


def cmd_status(args):
    _, live, records, rows = build_status()
    if args.json:
        print(json.dumps(rows, indent=2, default=str))
        return 0
    rung_names = [r["name"] for r in live]
    w = max(len(r["component"]) for r in rows) if rows else 12
    # The grade column is as wide as the grades this repository actually has,
    # and ten characters of digest tell two apart: the table then fits an
    # 80-column terminal. The JSON carries every digest whole.
    g = max([len(r["grade"]) for r in rows] + [5]) if rows else 5
    header = f"{'component':<{w}}  {'grade':<{g}}" + "".join(f"  {n:<10}" for n in rung_names)
    print(header)
    print("-" * len(header))
    for r in sorted(rows, key=lambda x: x["component"]):
        cells = []
        for n in rung_names:
            cell = r["rungs"].get(n) or {}
            v = cell.get("value")
            # No digest: the tag is what distinguishes it. The last ten
            # characters of the whole ref cut a tag in half
            # (`grant-keyd:sha-0751afc` read as `ha-0751afc`), and a `sha-`
            # prefix is a convention, not part of the identifier.
            short = ("absent" if cell.get("state") == "absent"
                     else "-" if v is None
                     else v.split("@sha256:")[-1][:10] if "@sha256:" in v
                     else v.rsplit(":", 1)[-1].removeprefix("sha-")[:10])
            cells.append(f"  {short:<10}")
        print(f"{r['component']:<{w}}  {r['grade']:<{g}}" + "".join(cells))
    open_recs = [r for r in records if r.get("state") == "candidate" and "_error" not in r]
    if open_recs:
        print(f"\nopen records ({len(open_recs)}):")
        for r in open_recs:
            print(f"  {r['component']}: {r.get('class','ladder')} opened {r.get('opened')} "
                  f"ref …{str(r.get('candidate_ref'))[-19:]}  ({r['_path'].relative_to(REPO)})")
    return 0


def record_dir(comp):
    d = RECORDS / comp
    d.mkdir(parents=True, exist_ok=True)
    return d


def preprod_prod(live):
    by = {r["name"]: r for r in live}
    if "preprod" not in by or "prod" not in by:
        die("ladder must have live preprod and prod rungs")
    return by["preprod"], by["prod"]


def cmd_open(args):
    ladder, _, live = load_ladder()
    pins = load_pins()
    comp = pins.get("components", {}).get(args.component) or die(f"unknown component {args.component!r}")
    if not DIGEST_RE.search(args.ref):
        die("candidate ref must be digest-pinned (…@sha256:…) — same rule as fleet-pins")
    pre, prod = preprod_prod(live)
    today = dt.date.today().isoformat()
    path = record_dir(args.component) / f"{today.replace('-','')}-{args.ref.split(':')[-1][:12]}.yaml"
    rec = {
        "component": args.component,
        "class": "ladder",
        "candidate_ref": args.ref,
        "previous_fleet_ref": comp.get("fleet"),
        "opened": today,
        "reason": args.reason,
        "evidence": {"validation": "", "preprod_convergence": ""},
        "state": "candidate",
        "graduated": None,
        "operator_ack": None,
    }
    if rec["previous_fleet_ref"] == rec["candidate_ref"]:
        die(
            "previous_fleet_ref would equal candidate_ref — the record's rollback "
            "pointer would name the ref being deployed. Open the record BEFORE "
            "moving fleet:."
        )
    path.write_text(yaml.safe_dump(rec, sort_keys=False))
    print(f"record written: {path.relative_to(REPO)}")
    print(f"\nnow make this edit in ops/pins/fleet-pins.yaml under components.{args.component}:")
    print(f"    deviations:\n      {pre['goatnet']}:\n        value: {args.ref}\n"
          f"        reason: \"RC ({today}): {args.reason} — record {path.name}\"")
    print("\nthen: python3 ops/pins/generate-pins.py && deploy to preprod && fill "
          "evidence fields in the record as it soaks. `graduate.py check` gates the PR.")
    return 0


def gate_id(g):
    return g["id"] if isinstance(g, dict) else g


def runtime_health_settings(hop, gate):
    """(max_memory_fraction, min_days, require_memory_limit) for one
    runtime-health gate entry. min_days defaults to the hop's own soak gate, so
    the capture is taken no earlier than the soak's end; a hop with no soak
    gate (dev->preprod) waits one day unless the entry says otherwise."""
    g = gate if isinstance(gate, dict) else {}
    soak = next((x for x in hop.get("gates", []) if gate_id(x) == "soak"), None)
    soak_days = (soak.get("min_days", 1) if isinstance(soak, dict) else 1) if soak is not None else 1
    return (g.get("max_memory_fraction", RUNTIME_HEALTH_MAX_MEMORY_FRACTION),
            g.get("min_days", soak_days),
            g.get("require_memory_limit", True) is not False)


def runtime_health_override(rec, rung):
    """The record's override for this rung when it names who AND why; else None."""
    o = rec.get("runtime_health_override")
    o = o.get(rung) if isinstance(o, dict) else None
    if isinstance(o, dict) and str(o.get("reason") or "").strip() and str(o.get("by") or "").strip():
        return o
    return None


def _utc(value):
    """An ISO-8601 time from a record (string, or a datetime YAML already
    parsed), as an aware datetime; None when unreadable."""
    if isinstance(value, dt.datetime):
        return value if value.tzinfo else value.replace(tzinfo=dt.timezone.utc)
    try:
        t = dt.datetime.fromisoformat(str(value).strip().replace("Z", "+00:00"))
    except (TypeError, ValueError):
        return None
    return t if t.tzinfo else t.replace(tzinfo=dt.timezone.utc)


def grade_runtime_health(block, rung, candidate_ref, max_fraction, min_days, require_limit):
    """Every reason one captured runtime_health block fails the gate. The
    verdict is recomputed from the numbers; a `verdict:` field in the block is
    never trusted. A failure starting `runtime-health awaiting` is a capture
    not taken yet (or taken before the soak ended); the rest are measurements."""
    if block is None:
        return [f"runtime-health awaiting: no runtime_health.{rung} block "
                f"(ops/release/rung-health.py --write-record, at the soak's end)"]
    if not isinstance(block, dict):
        return [f"runtime-health: runtime_health.{rung} is not a mapping"]
    fails = []
    captured, deployed = _utc(block.get("captured")), _utc(block.get("deployed"))
    if captured is None or deployed is None:
        fails.append(f"runtime-health: runtime_health.{rung} has no readable captured/deployed time")
    else:
        hours = int((captured - deployed).total_seconds() // 3600)
        if hours < min_days * 24:
            fails.append(f"runtime-health awaiting: runtime_health.{rung} was captured {hours} h "
                         f"after deploy, needs >= {min_days * 24} h (the soak)")
    m = DIGEST_RE.search(str(candidate_ref or ""))
    if m:
        digest = m.group(0)[1:]
        if digest not in [str(d) for d in (block.get("digests") or [])]:
            fails.append(f"runtime-health: runtime_health.{rung} was not captured on the "
                         f"candidate digest {digest[7:19]}")
    for key, what in (("restarts", "restarted"), ("oom_kills", "OOM-killed")):
        n = block.get(key)
        if not isinstance(n, int) or isinstance(n, bool):
            fails.append(f"runtime-health: {rung} {key} unknown — the capture could not read it")
        elif n > 0:
            fails.append(f"runtime-health: {rung} {what} {n} time(s) since deploy")
    frac = block.get("peak_memory_fraction")
    if frac is None or isinstance(frac, bool) or not isinstance(frac, (int, float)):
        if require_limit:
            fails.append(f"runtime-health: {rung} peak memory has no limit to grade against "
                         f"(set a memory limit, or require_memory_limit: false on the gate)")
    elif frac > max_fraction:
        fails.append(f"runtime-health: {rung} peak memory reached {frac} of its limit, "
                     f"ceiling {max_fraction}")
    return fails


def runtime_health_failures(rec, hop, gate):
    """The runtime-health gate for this hop: graded on the LOWER rung (the
    hop's `from`), satisfied by a recorded override for that rung."""
    rung = hop.get("from")
    if runtime_health_override(rec, rung):
        return []
    max_fraction, min_days, require_limit = runtime_health_settings(hop, gate)
    blocks = rec.get("runtime_health")
    block = blocks.get(rung) if isinstance(blocks, dict) else None
    return grade_runtime_health(block, rung, rec.get("candidate_ref"),
                                max_fraction, min_days, require_limit)


def gate_failures(rec, hop, today):
    fails = []
    for g in hop.get("gates", []):
        gid = g["id"] if isinstance(g, dict) else g
        if gid == "candidate-record":
            if rec.get("state") != "candidate":
                fails.append("record is not in state candidate")
        elif gid == "soak":
            mind = (g.get("min_days", 1) if isinstance(g, dict) else 1)
            opened = dt.date.fromisoformat(str(rec.get("opened")))
            if (today - opened).days < mind:
                fails.append(f"soak: opened {opened}, needs >= {mind} day(s)")
        elif gid == "convergence-evidence":
            if not (rec.get("evidence") or {}).get("preprod_convergence"):
                fails.append("evidence.preprod_convergence is empty (name the goat "
                             "health consistency band / console render / M-doc)")
        elif gid == "validation-evidence":
            if not (rec.get("evidence") or {}).get("validation"):
                fails.append("evidence.validation is empty")
        elif gid == "operator-ack":
            pass  # supplied at graduate time via --ack
        elif gid == "runtime-health":
            fails.extend(runtime_health_failures(rec, hop, g))
    return fails


def cmd_graduate(args):
    ladder, rungs, live = load_ladder()
    records = load_records()
    # The ladder candidate, never a hotfix record: a hotfix (class: hotfix)
    # stays `candidate` as the backfill-owed marker and sorts after a same-day
    # ladder record by name, so the unqualified "latest candidate" was the
    # hotfix — and its empty evidence read as the ladder candidate's.
    rec = latest_record(records, args.component, state="candidate", cls="ladder")
    if rec is None:
        # A fastlane's backfill (ADR 1127, F-514): the hotfix record IS the
        # candidate once its evidence is filled and its soak has run, and the
        # ref is already on every rung, so `open` (which refuses when fleet:
        # already carries the ref) is not the path. Graduating the hotfix
        # record itself is how the debt closes; the same hop gates apply.
        rec = latest_record(records, args.component, state="candidate", cls="hotfix") or \
            die(f"no open ladder candidate or hotfix record for {args.component}")
        print(f"graduate: closing the fastlane backfill on {rec['_path'].relative_to(REPO)} "
              f"(hotfix record; the same preprod->prod gates apply)")
    pre, prod = preprod_prod(live)
    hop = next((h for h in ladder.get("hops", [])
                if h.get("from") == "preprod" and h.get("to") == "prod"), None) or \
        die("no preprod->prod hop in ladder")
    # THE REFUSAL (ADR 1125). A component inside an OPEN graduation set may not
    # graduate alone: the set is what was validated together, and letting one
    # member move is exactly how a train arrives in pieces. `graduate-set` is
    # the way through, and it is named here rather than left to be discovered.
    sets = load_sets()
    owning = open_set_for(sets, args.component)
    if owning and not getattr(args, "_from_set", False):
        print(f"graduate: REFUSING — {args.component} is a member of the OPEN graduation "
              f"set '{owning.get('name')}' ({owning['_path'].relative_to(REPO)}).",
              file=sys.stderr)
        print("  The set moves together or not at all; graduating one member is how a "
              "validated train arrives in pieces.", file=sys.stderr)
        print(f"  Members: "
              f"{', '.join(m.get('component', '?') for m in set_members(owning))}",
              file=sys.stderr)
        print(f"\n  graduate the set:  python3 ops/release/graduate.py graduate-set "
              f"{owning.get('name')} --ack '<name>'", file=sys.stderr)
        print("  or take this component out of the set (edit the file, in a reviewed PR) "
              "if it genuinely moves alone.", file=sys.stderr)
        return 1
    override = getattr(args, "override_runtime_health", None)
    if override is not None:
        if not str(override).strip():
            die("--override-runtime-health needs a reason")
        if not args.ack:
            die("--override-runtime-health needs --ack '<name>' — an override names who decided")
        rt = [f for g in hop.get("gates", []) if gate_id(g) == "runtime-health"
              for f in runtime_health_failures(rec, hop, g)]
        if rt:
            rec.setdefault("runtime_health_override", {})
            if not isinstance(rec["runtime_health_override"], dict):
                rec["runtime_health_override"] = {}
            rec["runtime_health_override"][hop.get("from")] = {
                "by": args.ack, "reason": str(override).strip(),
                "recorded": dt.date.today().isoformat(), "failures": rt}
            print(f"graduate: runtime-health overridden for {hop.get('from')} by {args.ack}: "
                  f"{str(override).strip()}")
    fails = gate_failures(rec, hop, dt.date.today())
    needs_ack = any((g["id"] if isinstance(g, dict) else g) == "operator-ack"
                    for g in hop.get("gates", []))
    if needs_ack and not args.ack:
        fails.append("operator-ack gate: pass --ack '<name>'")
    if fails:
        print(f"graduate: {args.component} is NOT ready:", file=sys.stderr)
        for f in fails:
            print(f"  - {f}", file=sys.stderr)
        return 1
    rec["state"] = "graduated"
    rec["graduated"] = dt.date.today().isoformat()
    rec["operator_ack"] = args.ack
    path = rec.pop("_path")
    path.write_text(yaml.safe_dump(rec, sort_keys=False))
    print(f"record graduated: {path.relative_to(REPO)}")
    print(f"\nnow make this edit in ops/pins/fleet-pins.yaml under components.{args.component}:")
    print(f"    fleet: {rec['candidate_ref']}")
    print(f"    # and DELETE the {pre['goatnet']} RC deviation added by `open`"
          f" (and any {prod['goatnet']} hotfix deviation for this ref)")
    print("\nthen: python3 ops/pins/generate-pins.py && PR && deploy prod && verify "
          "with the consistency surface. `check` refuses a tree where the "
          "graduated record and fleet value disagree (C5).")
    return 0


def cmd_graduate_set(args):
    """Graduate every member of a set in one act, or refuse and name every
    member that is not ready. There is no partial outcome: a set that
    half-graduated would be the state this whole increment exists to prevent."""
    ladder, rungs, live = load_ladder()
    records = load_records()
    sets = load_sets()
    doc = next((d for d in sets if "_error" not in d and d.get("name") == args.name), None) or \
        die(f"no graduation set named {args.name!r} in ops/release/sets/")
    if doc.get("state") != "open":
        die(f"set {args.name!r} is {doc.get('state')!r}, not open")
    pre, prod = preprod_prod(live)
    hop = next((h for h in ladder.get("hops", [])
                if h.get("from") == "preprod" and h.get("to") == "prod"), None) or \
        die("no preprod->prod hop in ladder")
    today = dt.date.today()

    # EVERY blocker, not the first: fixing one and rediscovering the next is
    # the readiness experience this tool exists to replace.
    blockers = set_blockers(doc, records, hop, today)
    needs_ack = any((g["id"] if isinstance(g, dict) else g) == "operator-ack"
                    for g in hop.get("gates", []))
    if needs_ack and not args.ack:
        blockers.append("operator-ack gate: pass --ack '<name>' (one ack covers the set)")
    if blockers:
        print(f"graduate-set: {args.name} is NOT ready ({len(blockers)} blocker(s)):",
              file=sys.stderr)
        for b in blockers:
            print(f"  - {b}", file=sys.stderr)
        return 1

    for m in set_members(doc):
        rec = latest_record(records, m["component"], state="candidate", ref=m["ref"])
        rec["state"] = "graduated"
        rec["graduated"] = today.isoformat()
        rec["operator_ack"] = args.ack
        rec["graduated_with_set"] = args.name
        path = rec.pop("_path")
        path.write_text(yaml.safe_dump(rec, sort_keys=False))
        print(f"record graduated: {path.relative_to(REPO)}")
    doc["state"] = "graduated"
    doc["graduated"] = today.isoformat()
    doc["operator_ack"] = args.ack
    spath = doc.pop("_path")
    spath.write_text(yaml.safe_dump(doc, sort_keys=False))
    print(f"set graduated: {spath.relative_to(REPO)}")

    print("\nnow make these edits in ops/pins/fleet-pins.yaml, IN ONE COMMIT — "
          "splitting them re-creates the partial promotion this set prevents:")
    for m in set_members(doc):
        print(f"    components.{m['component']}.fleet: {m['ref']}")
    print(f"    # and DELETE each member's {pre['goatnet']} RC deviation")
    print("\nthen: python3 ops/pins/generate-pins.py && PR && deploy prod && verify.")
    return 0


def cmd_hotfix(args):
    ladder, _, live = load_ladder()
    pins = load_pins()
    comp = pins.get("components", {}).get(args.component) or die(f"unknown component {args.component!r}")
    if not DIGEST_RE.search(args.ref):
        die("hotfix ref must be digest-pinned (…@sha256:…)")
    if not args.ack:
        die("hotfix requires --ack '<name>' — it is a prod change")
    pre, prod = preprod_prod(live)
    today = dt.date.today().isoformat()
    path = record_dir(args.component) / f"{today.replace('-','')}-hotfix-{args.ref.split(':')[-1][:12]}.yaml"
    rec = {
        "component": args.component,
        "class": "hotfix",
        "candidate_ref": args.ref,
        "previous_fleet_ref": args.previous_ref or comp.get("fleet"),
        "opened": today,
        "reason": args.reason,
        "evidence": {"validation": "", "preprod_convergence": ""},
        "state": "candidate",
        "graduated": None,
        "operator_ack": args.ack,
    }
    # A caller that has ALREADY moved fleet: must pass --previous-ref; reading
    # fleet: would otherwise record the ref being deployed as the ref to roll
    # back to, and the record's rollback pointer would be useless. Found on the
    # first live fastlane (id-admin-console sha-4441fb2 ->
    # sha-e1e5358): ops/scripts/fastlane.sh rewrites fleet-pins.yaml at its step
    # 3 and opens this record at its step 6, so previous_fleet_ref came out
    # equal to candidate_ref. Fail rather than write a record that lies.
    if rec["previous_fleet_ref"] == rec["candidate_ref"]:
        die(
            "previous_fleet_ref would equal candidate_ref — the record's rollback "
            "pointer would name the ref being deployed. Pass --previous-ref with "
            "the ref you moved away from."
        )
    path.write_text(yaml.safe_dump(rec, sort_keys=False))
    print(f"hotfix record written: {path.relative_to(REPO)} (backfill owed — "
          f"status shows hotfix-backfill-owed until this ref graduates normally)")
    print(f"\nnow make this edit in ops/pins/fleet-pins.yaml under components.{args.component}:")
    print(f"    deviations:\n      {prod['goatnet']}:\n        value: {args.ref}\n"
          f"        reason: \"HOTFIX ({today}, ack {args.ack}): {args.reason} — record {path.name}\"")
    print("\nbackfill: deploy the same ref to preprod, fill evidence, then "
          "`graduate.py graduate` folds it to fleet and clears both deviations.")
    return 0


# ---------------------------------------------------------------------------
# readiness — the ladder as a document a surface can render (release-graduation.md §8).
#
# One rule governs this whole section: it DERIVES, it does not decide. Every
# grade comes from `build_status`, every blocker from the same `gate_failures`
# `graduate` runs, and anything the tree cannot answer is emitted as an
# explicit unknown with the reason attached — never as a default, a zero or an
# empty string that a renderer could mistake for a measurement.
# ---------------------------------------------------------------------------

READINESS_SCHEMA = "release-readiness-v1"
ROLES = REPO / "ops/ansible/roles"
SHA_TAG_RE = re.compile(r":sha-([0-9a-f]{7,40})@")


def live_hop(ladder):
    """The preprod->prod hop, or None. The gates `graduate` enforces."""
    return next((h for h in ladder.get("hops", [])
                 if h.get("from") == "preprod" and h.get("to") == "prod"
                 and h.get("status") == "live"), None)


def git_out(*args):
    """(ok, text). A checkout is not guaranteed — the document is generated on
    whatever host runs the deploy — so every git-derived field is optional."""
    try:
        out = subprocess.run(["git", "-C", str(REPO), *args], capture_output=True,
                             text=True, timeout=10)
    except (OSError, subprocess.SubprocessError) as e:  # noqa: BLE001
        return False, str(e)
    if out.returncode != 0:
        return False, (out.stderr or out.stdout).strip()
    return True, out.stdout.strip()


def image_workflow(comp_def):
    """The component's DECLARED image workflow and the `paths:` filter that
    decides whether a push builds it. That filter is the tree's own answer to
    "what source produces a new image for this component" — it already lives in
    one home (the workflow), so this reads it rather than introducing a second
    mapping. Returns {"workflow", "paths", "why"}: `paths` is None whenever the
    question could not be answered, and `why` then says which way it failed."""
    if "image_workflow" not in comp_def:
        return {"workflow": None, "paths": None,
                "why": "fleet-pins.yaml declares no `image_workflow:` for this "
                       "component, so the tree names no build-input path set"}
    rel = comp_def["image_workflow"]
    if rel is None:
        return {"workflow": None, "paths": None,
                "why": "fleet-pins.yaml declares this component has no image "
                       "workflow in this repo"}
    rel = str(rel)
    paths, why = workflow_push_paths(rel)
    return {"workflow": rel, "paths": paths, "why": why}


def workflow_push_paths(rel):
    """(paths, why) — exactly one is not None. A filter this function cannot
    evaluate is a stated unknown, never an empty path list: an empty list would
    silently count every commit as irrelevant, which is the shape of a false
    green."""
    path = REPO / rel
    if not path.is_file():
        return None, f"declared image workflow {rel} is not in this checkout"
    try:
        wf = load_yaml(path) or {}
    except yaml.YAMLError as e:
        return None, f"{rel} did not parse as YAML: {e}"
    # A bare `on:` key resolves to the boolean True under YAML 1.1, which is
    # what PyYAML implements and what every GitHub workflow in this repo hits.
    on = wf.get("on", wf.get(True))
    push = (on or {}).get("push") if isinstance(on, dict) else None
    if not isinstance(push, dict):
        return None, f"{rel} declares no `on.push` trigger to read a filter from"
    paths = push.get("paths")
    if paths is None:
        if push.get("paths-ignore"):
            return None, (f"{rel} filters with `paths-ignore` — an exclusion "
                          "this tool does not evaluate")
        return None, (f"{rel} declares no `on.push.paths` filter, so every push "
                      "to its branches builds and no subset of the tree is this "
                      "component's build input")
    negated = [str(p) for p in paths if str(p).startswith("!")]
    if negated:
        return None, (f"{rel} `paths:` carries negated pattern(s) "
                      f"{', '.join(negated)} — this tool evaluates no exclusions")
    return [str(p) for p in paths], None


_COMPONENT_COMMITS_CACHE = {}


def component_commits(commit, head, paths):
    """Commits between the pinned build's commit and the generating HEAD that
    touch the component's declared build inputs. `:(glob)` pathspecs so `**`
    means what the workflow's filter means by it."""
    key = (commit, head, tuple(paths))
    if key not in _COMPONENT_COMMITS_CACHE:
        spec = [f":(glob){p}" for p in paths]
        ok, count = git_out("rev-list", "--count", f"{commit}..{head}", "--", *spec)
        if not ok or not count.isdigit():
            out = {"known": False,
                   "why": f"git could not count commits over the declared paths: {count}"}
        else:
            n = int(count)
            newest = None
            if n:
                ok2, line = git_out("log", "-1", "--format=%h%x1f%cI%x1f%s",
                                    f"{commit}..{head}", "--", *spec)
                parts = line.split("\x1f") if ok2 else []
                if len(parts) == 3:
                    newest = {"sha": parts[0], "date": parts[1], "subject": parts[2]}
            out = {"known": True, "count": n, "newest": newest}
        _COMPONENT_COMMITS_CACHE[key] = out
    return dict(_COMPONENT_COMMITS_CACHE[key])


def component_commit_facts(commit, head, wf):
    """The per-rung "is there a build nothing pins?" reading: how many commits
    since THIS rung's pinned build touched the paths its image workflow builds
    on. A count of 0 is a measurement; every other outcome is an unknown that
    carries its reason."""
    wf = wf or image_workflow({})
    base = {"workflow": wf["workflow"], "paths": wf["paths"] or []}
    if wf["paths"] is None:
        return {**base, "known": False, "why": wf["why"]}
    if not commit:
        return {**base, "known": False,
                "why": "no build commit resolvable in this checkout to count from "
                       "(see the build note for which way the ref failed)"}
    if not head:
        return {**base, "known": False,
                "why": "the generating checkout resolved no HEAD to count to"}
    return {**base, **component_commits(commit, head, wf["paths"])}


def build_facts(ref, head, wf=None):
    """What the tree knows about the build a pinned ref came from: the
    `sha-<short>` tag's commit, its date, how far the generating checkout's HEAD
    has moved since, and — the reading that is actually about this component —
    how many of those commits touched the paths its image workflow builds on.
    `commits_on_head_since` counts ALL commits on HEAD and is labelled as such;
    `component_commits` is the attributed number."""
    def out(facts, commit=None):
        facts["component_commits"] = component_commit_facts(commit, head, wf)
        return facts

    if not ref:
        return out({"tag": None, "known": False, "why": "no ref pinned"})
    m = SHA_TAG_RE.search(ref)
    if not m:
        return out({"tag": None, "known": False,
                    "why": "ref carries no sha-<short> tag (digest-only pin)"})
    tag = f"sha-{m.group(1)}"
    ok, commit = git_out("rev-parse", "--verify", f"{m.group(1)}^{{commit}}")
    if not ok:
        return out({"tag": tag, "known": False,
                    "why": "the build commit is not in this checkout"})
    ok, when = git_out("show", "-s", "--format=%cI", commit)
    ok2, ahead = git_out("rev-list", "--count", f"{commit}..{head}") if head else (False, "")
    return out({
        "tag": tag,
        "known": True,
        "commit": commit,
        "commit_date": when if ok else None,
        "commits_on_head_since": int(ahead) if ok2 and ahead.isdigit() else None,
    }, commit=commit)


def ledger_rows(name, comp_def):
    """The component's ADR 1060 in-repo ledger, joined by the DECLARED
    `ledger_component` key (never inferred from `var:` — fleet-pins.yaml says
    why). Returns (status, rows, path)."""
    if "ledger_component" in comp_def and comp_def["ledger_component"] is None:
        return "no-ledger-coverage", [], None
    ledger = comp_def.get("ledger_component") or name
    path = ROLES / ledger / "deploy-history.yml"
    if not path.is_file():
        return "ledger-file-missing", [], str(path.relative_to(REPO))
    try:
        doc = load_yaml(path) or {}
    except yaml.YAMLError:
        return "ledger-unreadable", [], str(path.relative_to(REPO))
    rows = [r for r in (doc.get("history") or []) if isinstance(r, dict)]
    if not rows:
        return "no-history-rows", [], str(path.relative_to(REPO))
    return "rows", rows, str(path.relative_to(REPO))


def digest_of(ref):
    """The `sha256:…` identity of an image ref, or None. The join key between
    the pin table and the deploy ledger MUST be the digest: the table carries
    the literal `{{ ghcr_org }}` template while the ledger records the resolved
    org, so the two never compare equal as whole refs even for the same
    artifact."""
    if not isinstance(ref, str):
        return None
    m = DIGEST_RE.search(ref)
    return ref[m.start() + 1:m.end()] if m else None


def ledger_facts(name, comp_def, pinned_refs):
    """Join the pins to the deploy ledger by digest. `pin_deployed` is the
    merged-but-not-deployed signal this tree CAN answer: a pinned artifact with
    no ledger row was never recorded as deployed anywhere. `None` means the
    question could not be asked (no digest to join on, or no ledger)."""
    status, rows, path = ledger_rows(name, comp_def)
    out = {"status": status, "path": path, "newest": None,
           "pin_deployed": {k: None for k in pinned_refs}}
    if status != "rows":
        return out
    # merged_at is a bare YAML timestamp in some ledgers and a quoted string in
    # others; both sort correctly as text within one file, which is the scope
    # of this comparison.
    dated = [r for r in rows if r.get("merged_at") is not None]
    newest = max(dated, key=lambda r: str(r["merged_at"])) if dated else rows[0]
    out["newest"] = {
        "sha": newest.get("sha"),
        "image": newest.get("image"),
        "digest": digest_of(newest.get("image")),
        "pr": newest.get("pr"),
        "merged_at": str(newest.get("merged_at")) if newest.get("merged_at") else None,
        "ordered_by": "merged_at" if dated else "file order (no merged_at on any row)",
    }
    deployed = {d for d in (digest_of(r.get("image")) for r in rows) if d}
    out["pin_deployed"] = {
        k: (digest_of(v) in deployed if digest_of(v) else None)
        for k, v in pinned_refs.items()
    }
    pinned_digests = {d for d in (digest_of(v) for v in pinned_refs.values()) if d}
    out["newest_is_pinned"] = (out["newest"]["digest"] in pinned_digests
                               if out["newest"]["digest"] else None)
    return out


def soak_waiver(rec):
    """The record's explicit `soak_waiver:` block, or None. A waiver states WHO
    and WHY; `by: null` is a waiver whose author the record does not name, and
    that reads as unknown attribution rather than as an unattributed decision
    being quietly accepted."""
    w = rec.get("soak_waiver")
    if not isinstance(w, dict):
        return None
    by = str(w["by"]).strip() if w.get("by") else None
    return {
        "by": by,
        "attribution": "named" if by else "unknown",
        "reason": str(w["reason"]).strip() if w.get("reason") else None,
    }


def soak_facts(rec, hop, today):
    """The soak clock, measured from the record's `opened` date — the only
    date the record carries. It is NOT a deploy clock: nothing in the schema
    records when the candidate reached preprod, so an RC opened before it
    deployed reads as having soaked longer than it ran.

    A recorded waiver is its own state: an operator decided the window did not
    have to elapse, which is a different fact from a window that has not
    elapsed yet. It does not satisfy the gate — `gate_failures` (what
    `graduate` refuses on) is untouched by it."""
    gate = next((g for g in (hop or {}).get("gates", [])
                 if (g["id"] if isinstance(g, dict) else g) == "soak"), None)
    waiver = soak_waiver(rec)
    if gate is None:
        return {"state": "no-soak-gate", "required_days": None}
    required = gate.get("min_days", 1) if isinstance(gate, dict) else 1
    try:
        opened = dt.date.fromisoformat(str(rec.get("opened")))
    except (TypeError, ValueError):
        why = f"record `opened` is not a date: {rec.get('opened')!r}"
        if waiver:
            return {"state": "waived", "required_days": required,
                    "why": why, "waiver": waiver}
        return {"state": "unknown", "required_days": required, "why": why}
    elapsed = (today - opened).days
    met = elapsed >= required
    facts = {
        "state": "met" if met else ("waived" if waiver else "soaking"),
        "required_days": required,
        "opened": opened.isoformat(),
        "elapsed_days": elapsed,
        "remaining_days": max(required - elapsed, 0),
        "clock": "record `opened` date",
    }
    if waiver:
        facts["waiver"] = waiver
    return facts


def candidate_readiness(rec, hop, today):
    """ready / soaking / soak-waived / blocked, and WHAT blocks it —
    `gate_failures`, the same call `graduate` gates on, plus the ack the hop
    asks for. `soak-waived` is NOT `ready`: the blockers stay verbatim because
    `graduate` still refuses on the unmet gate; the state only stops a recorded
    operator decision from rendering as a window nobody has looked at."""
    blockers = gate_failures(rec, hop or {}, today)
    ack_required = any((g["id"] if isinstance(g, dict) else g) == "operator-ack"
                       for g in (hop or {}).get("gates", []))
    if ack_required and not rec.get("operator_ack"):
        blockers.append("operator-ack gate: the record names no operator_ack")
    waiver = soak_waiver(rec)
    if not blockers:
        state = "ready"
    elif all(b.startswith("soak") or b.startswith("runtime-health awaiting") for b in blockers):
        state = "soak-waived" if waiver else "soaking"
    else:
        state = "blocked"
    return {
        "state": state,
        "blockers": blockers,
        "ack_required": ack_required,
        "operator_ack": rec.get("operator_ack"),
        "soak_waiver": waiver,
    }


def component_readiness(row, comp_def, records, live, hop, today, head, goatnets):
    """One component's row of the document. `row` is `build_status`'s graded
    row — this function adds no grade of its own."""
    name = row["component"]
    devs = comp_def.get("deviations") or {}
    wf = image_workflow(comp_def)
    rungs = []
    pinned = {"fleet": comp_def.get("fleet")}
    for r in live:
        cell = row["rungs"].get(r["name"]) or {}
        dev = devs.get(r["goatnet"]) or {}
        value = cell.get("value")
        pinned[r["name"]] = value
        rungs.append({
            "rung": r["name"],
            "goatnet": r["goatnet"],
            "state": cell.get("state", "unresolved"),
            "value": value,
            "deviates_from_fleet": bool(dev),
            "deviation_reason": dev.get("reason"),
            "build": build_facts(value, head, wf),
        })
    # Member-level deviations are intra-goatnet (node parity's axis, not the
    # ladder's) — carried so the view can show them rather than drop them.
    member_devs = [{"member": k, "reason": (v or {}).get("reason"),
                    "absent": bool((v or {}).get("absent")), "value": (v or {}).get("value")}
                   for k, v in devs.items() if k not in goatnets]
    rec = latest_record(records, name, state="candidate")
    candidate = None
    if rec is not None:
        candidate = {
            "path": str(rec["_path"].relative_to(REPO)),
            "class": rec.get("class", "ladder"),
            "candidate_ref": rec.get("candidate_ref"),
            "previous_fleet_ref": rec.get("previous_fleet_ref"),
            "opened": rec.get("opened"),
            "reason": rec.get("reason"),
            "evidence": {k: bool((rec.get("evidence") or {}).get(k))
                         for k in ("validation", "preprod_convergence")},
            "note": rec.get("note"),
            "soak": soak_facts(rec, hop, today),
            "readiness": candidate_readiness(rec, hop, today),
        }
    last = latest_record(records, name, state="graduated")
    return {
        "component": name,
        "class": row["class"],
        "grade": row["grade"],
        "pairs": row["pairs"],
        "var": comp_def.get("var"),
        "artifact_type": comp_def.get("artifact_type", "image"),
        "fleet": comp_def.get("fleet"),
        "image_workflow": wf,
        "rungs": rungs,
        "member_deviations": member_devs,
        "candidate": candidate,
        "readiness_state": (candidate["readiness"]["state"] if candidate
                            else "not-applicable" if row["class"] == "exempt"
                            else "no-candidate"),
        "last_graduated": ({"ref": last.get("candidate_ref"), "on": last.get("graduated"),
                            "ack": last.get("operator_ack"),
                            "soak_waiver": soak_waiver(last),
                            "path": str(last["_path"].relative_to(REPO))} if last else None),
        "ledger": ledger_facts(name, comp_def, pinned),
    }


def build_readiness(today=None):
    ladder, live, records, rows = build_status()
    pins = load_pins()
    today = today or dt.date.today()
    hop = live_hop(ladder)
    goatnets = set((pins.get("goatnets") or {}).keys())
    ok, head = git_out("rev-parse", "HEAD")
    head = head if ok else None
    comps = pins.get("components") or {}
    doc = {
        "schema": READINESS_SCHEMA,
        "generated_unix_ms": int(dt.datetime.now(dt.timezone.utc).timestamp() * 1000),
        "generated_date": today.isoformat(),
        "source": {
            "head_commit": head,
            "git": "available" if head else "unavailable — build ages are unknown",
            "note": "generated from a checkout at deploy time; it does not "
                    "refresh itself, so its age IS the age of its facts",
        },
        "ladder": {
            "rungs": [{"name": r["name"], "goatnet": r["goatnet"]} for r in live],
            "hop": ({"from": hop["from"], "to": hop["to"],
                     "gates": [g if isinstance(g, dict) else {"id": g}
                               for g in hop.get("gates", [])]} if hop else None),
        },
        "unknowable": [
            ("Whether an image was actually built and pushed: this tool contacts no CI "
            "and no registry. Per component it reports whether commits since the "
            "pinned build touched the paths that component's image workflow builds "
            "on — a strong \"a newer image exists that this rung does not run\", not "
            "a confirmed one."),
            ("Components whose build inputs the tree does not declare: no "
            "`image_workflow:` in fleet-pins.yaml, a workflow with no `on.push.paths` "
            "filter, or a filter this tool will not evaluate (exclusions, negated "
            "patterns). Those rows read unknown with the reason — never 0 commits."),
            ("CI run state: no committed artifact carries it and this tool contacts "
            "nothing; the newest deploy-ledger row's PR is the last build with in-tree "
            "evidence, which is not the same fact."),
            ("Whether a recorded soak waiver was justified: a `soak_waiver:` block is an "
            "operator's decision written down, verified by nothing. `graduate` still "
            "refuses on the unmet gate — the waiver records the decision, it does not "
            "override it."),
        ],
        "components": [component_readiness(row, comps.get(row["component"]) or {},
                                           records, live, hop, today, head, goatnets)
                       for row in sorted(rows, key=lambda x: x["component"])],
    }
    return doc


def _short(text, n=110):
    t = " ".join(str(text or "").split())
    return t if len(t) <= n else t[: n - 1] + "…"


def _narrate(name, r, rd, today):
    """The three sentences a person needs per record: what it is, what
    happens if it is left, what happens if it is acted on. Plain words, the
    consequence named, the prod change called a prod change."""
    cls = r.get("class") or "ladder"
    ref = (r.get("candidate_ref") or "").split("@")[0].split(":")[-1] or "?"
    reason = _short(r.get("reason"))
    ev = r.get("evidence") or {}
    missing = [k for k, v in ev.items() if not v]
    try:
        age = (today - dt.date.fromisoformat(str(r.get("opened")))).days
    except (TypeError, ValueError):
        age = None
    owner = rd.get("operator_ack") or "nobody named"
    if cls == "hotfix":
        validated = bool(ev.get("validation"))
        c9 = None; grandfathered = False
        try:
            opened = dt.date.fromisoformat(str(r.get("opened")))
            if opened >= HOTFIX_BACKFILL_CUTOFF:
                c9 = (opened + dt.timedelta(days=HOTFIX_BACKFILL_MAX_DAYS + 1)).isoformat()
            else:
                grandfathered = True
        except (TypeError, ValueError):
            pass  # unparseable opened date: no backfill cutoff
        what = (f"a fastlane debt: {name} {ref} went to EVERY goatnet on {r.get('opened')} with no soak "
                f"({reason}). Prod already runs it; "
                + ("its validation on dev and preprod is on record." if validated
                   else "nobody has validated it on record."))
        if rd["state"] == "ready":
            left = "the debt stays open and status keeps grading the component owed; nothing changes on any goatnet."
            acted = "graduate closes the record. No deploy — the ref is already everywhere. Bookkeeping, with evidence."
        elif rd["state"] in ("soaking", "soak-waived"):
            left = ("nothing until the runnable date; the debt is only bookkeeping now." if validated else
                    "the soak runs, but the record still owes its validation" +
                    (f" — CI check C9 blocks every PR from {c9}." if c9 else
                     "; nothing will ever force it (grandfathered), only this review shows it."))
            acted = (("on the runnable date, graduate closes the record. No deploy." if validated else
                      "run the mapped live rows on dev and preprod now (read-only probes, minutes), write PASS into "
                      "evidence.validation; on the runnable date graduate closes the record. No deploy."))
        else:
            left = ("prod keeps running a ref with no validation on record" +
                    (f"; CI check C9 blocks every PR from {c9}" if c9 and not validated
                     else ("; nothing will ever force it (grandfathered), so only this review shows it" if grandfathered
                           else "")) + ".")
            acted = ("run the mapped live rows on dev and preprod (read-only probes, minutes), write PASS into "
                     f"evidence.{'/'.join(missing) or 'validation'}; then graduate closes the record. No prod change.")
    else:
        what = (f"a release candidate: {name} {ref}, opened {r.get('opened')}"
                + (f" ({age} days ago)" if age is not None else "") + f", owner {owner}. {reason}")
        if rd["state"] == "ready":
            left = "preprod keeps running a ref prod does not; the candidate ages and its evidence goes stale."
            acted = ("graduate marks the record graduated and moves fleet: to this ref — PROD DEPLOYS IT on the next "
                     "converge. A real production change: read the record and the live state first.")
        elif rd["state"] in ("soaking", "soak-waived"):
            left = "the soak keeps running; nothing to do until the runnable date except fill the evidence."
            acted = ("fill the evidence it names now; on the runnable date graduate moves fleet: to this ref and "
                     "PROD DEPLOYS IT.")
        else:
            left = ("preprod runs a ref prod does not, with no evidence and no owner; it drifts until check C6 "
                    "abandons it when the rung moves — the candidate is quietly lost.")
            acted = ("decide it: either run the rows on dev and preprod and name an ack, then graduate (a PROD "
                     "DEPLOY), or abandon it with a reason so the ladder is honest. Either way the drift ends.")
    return {"what": what, "if_left": left, "if_acted": acted, "owner": owner, "age_days": age}


def build_review(today=None, ack="<you>"):
    """The start-of-day / end-of-day review: every open record sorted by what a
    person must DO about it, each with what it is, the consequence of leaving
    it, the consequence of acting on it, and the exact command. Read-only;
    nothing here graduates, folds or deploys (ADR 1177)."""
    today = today or dt.date.today()
    doc = build_readiness(today)
    act, soaking, blocked, attention = [], [], [], []
    for c in doc["components"]:
        name = c["component"]
        if c.get("grade") == "UNDECLARED-SKEW":
            hops = ", ".join(p.replace("->", " and ") for p, g in c["pairs"].items() if g == "UNDECLARED-SKEW")
            attention.append({"component": name,
                              "what": f"{name}: {hops} run different refs and no record says why.",
                              "if_left": "the ladder cannot say which goatnet is right; rollback has no pointer.",
                              "if_acted": "open a record for the intended ref, or fix the pin; nothing deploys.",
                              "command": "python3 ops/release/graduate.py status"})
        cand = c.get("candidate")
        for r in ([cand] if cand else []):
            rd = r["readiness"]; sk = r["soak"]
            base = {"component": name, "record": r["path"], "class": r["class"], "opened": r["opened"],
                    "ref": (r.get("candidate_ref") or "")[-19:], "state": rd["state"]}
            grad = f"python3 ops/release/graduate.py graduate {name} --ack {ack}"
            if r["class"] == "hotfix":
                grad += "  # closes the fastlane's hotfix record"
            if rd["state"] == "ready":
                item = {**base, "command": grad}
                item.update(_narrate(name, r, rd, today))
                act.append(item)
            elif rd["state"] in ("soaking", "soak-waived"):
                missing = [k for k, v in (r.get("evidence") or {}).items() if not v]
                runnable = None
                try:
                    runnable = (dt.date.fromisoformat(str(r["opened"])) + dt.timedelta(days=sk.get("required_days") or 1)).isoformat()
                except (TypeError, ValueError):
                    pass  # unparseable opened date: leave runnable unset
                item = {**base, "runnable": runnable, "remaining_days": sk.get("remaining_days"),
                        "fill_first": missing, "command": grad}
                item.update(_narrate(name, r, rd, today))
                soaking.append(item)
            else:
                item = {**base, "blockers": rd["blockers"], "command": grad}
                if r["class"] == "hotfix" and not (r.get("evidence") or {}).get("validation"):
                    try:
                        opened = dt.date.fromisoformat(str(r["opened"]))
                        if opened >= HOTFIX_BACKFILL_CUTOFF:
                            item["check_red_on"] = (opened + dt.timedelta(days=HOTFIX_BACKFILL_MAX_DAYS + 1)).isoformat()
                        else:
                            item["grandfathered"] = True
                    except (TypeError, ValueError):
                        pass  # unparseable opened date: no check_red_on
                    item["fill_first"] = ["run the live rows on dev and preprod (ops/validation/affected-flows.py "
                                          f"--component {name}; run-acceptance.py), write PASS into evidence.validation"]
                item.update(_narrate(name, {**r, "check_red_on": item.get("check_red_on")}, rd, today))
                blocked.append(item)
    chk = subprocess.run([sys.executable, str(REPO / "ops/release/graduate.py"), "check"],
                         capture_output=True, text=True)
    # `check` prints its failures on STDERR; reading stdout alone meant the
    # review never showed a red tree (found by the goatbuilder parity gate).
    check_lines = [l.strip() for l in (chk.stdout + chk.stderr).splitlines()
                   if l.strip().startswith("C")]
    prod_changes = sum(1 for i in act + soaking if "PROD DEPLOYS" in i.get("if_acted", ""))
    return {"schema": "release-review-v1", "date": today.isoformat(), "head": doc["source"]["head_commit"],
            "summary": {"act_now": len(act), "soaking": len(soaking), "blocked": len(blocked),
                        "attention": len(attention) + len(check_lines), "prod_changes_pending": prod_changes},
            "act_now": act, "soaking": soaking, "blocked": blocked, "attention": attention,
            "check": {"ok": chk.returncode == 0, "failures": check_lines}}


def _next_step(it):
    """One short clause: what to do next; PROD when it deploys to production."""
    st, cls = it.get("state"), it.get("class")
    tail = "PROD" if cls != "hotfix" else "no deploy"
    if st == "ready":
        return f"graduate → {tail}"
    if st in ("soaking", "soak-waived"):
        fill = "fill evidence → " if it.get("fill_first") else ""
        return f"{fill}graduate on {it.get('runnable') or '?'} → {tail}"
    blockers = it.get("blockers", [])
    needs = []
    if any("evidence" in b for b in blockers):
        needs.append("evidence")
    if any("operator-ack" in b for b in blockers):
        needs.append("ack")
    if any(b.startswith("runtime-health") for b in blockers):
        needs.append("runtime health")
    what = " + ".join(needs) or "gate"
    if cls == "hotfix":
        return f"rows → {what} → graduate → no deploy"
    return f"{what} → graduate → PROD"


def _due(it):
    """When something changes by itself: the runnable date, the day CI goes
    red, 'never' for a grandfathered debt, or a dash."""
    if it.get("runnable"):
        return it["runnable"]
    if it.get("check_red_on"):
        return f"CI {it['check_red_on'][5:]}"
    if it.get("grandfathered"):
        return "never"
    return "—"


def _age(it, today):
    try:
        d = (today - dt.date.fromisoformat(str(it.get("opened")))).days
        return f"{d}d"
    except (TypeError, ValueError):
        return "?"


def cmd_review(args):
    doc = build_review(ack=args.ack or "<you>")
    if args.json:
        print(json.dumps(doc, indent=2))
        return 0
    import shutil
    width = max(80, shutil.get_terminal_size((100, 24)).columns)
    today = dt.date.fromisoformat(doc["date"])
    sm = doc["summary"]
    head = (doc["head"] or "")[:9]
    rows = ([("READY", it) for it in doc["act_now"]] + [("SOAKING", it) for it in doc["soaking"]]
            + [("BLOCKED", it) for it in doc["blocked"]])
    only = getattr(args, "component", None)
    if only:
        rows = [(st, it) for st, it in rows if it["component"] == only]
    if not args.no_header:
        print(f"REVIEW  {doc['date']}  main {head or '?'}")
        print()
    print(f"LADDER  {sm['act_now']} ready · {sm['soaking']} soaking · {sm['blocked']} blocked · {sm['attention']} attention"
          + (f" · {sm['prod_changes_pending']} would deploy to PROD" if sm['prod_changes_pending'] else ""))
    if not rows and not doc["attention"] and not doc["check"]["failures"]:
        print("  nothing on the ladder needs a person today")
        return 0
    if not args.detail:
        cw = min(max((len(it["component"]) for _, it in rows), default=9), 22)
        fmt = f"  {{:<8}} {{:<{cw}}} {{:<10}} {{:>4}}  {{:<10}}  {{}}"
        print(fmt.format("STATE", "COMPONENT", "KIND", "AGE", "DUE", "NEXT"))
        for st, it in rows:
            # The ladder's own words are `candidate` and `fastlane`; this column
            # is for a reader (docs/design/goatbuilder-output.md rule 4).
            kind = "hotfix" if it["class"] == "hotfix" else "release"
            step = _next_step(it)
            line = fmt.format(st, it["component"][:cw], kind, _age(it, today), _due(it), step)
            if len(line) < width:
                print(line)
                continue
            # NEXT is the only place a row says what to do, so it moves to its
            # own line rather than being cut in half ("evidence + ack → gr").
            print(fmt.format(st, it["component"][:cw], kind, _age(it, today), _due(it), "").rstrip())
            import textwrap as _tw
            for l in _tw.wrap(f"next: {step}", width=width - 1, initial_indent=" " * 11, subsequent_indent=" " * 17):
                print(l)
        for it in doc["attention"]:
            print((f"  {'SKEW':<8} {it['component'][:cw]:<{cw}} {'tree':<10} {'':>4}  {'—':<10}  fix the pin or open a record")[:width - 1])
        for f in doc["check"]["failures"]:
            print((f"  {'CHECK':<8} {'graduate.py':<{cw}} {'tree':<10} {'':>4}  {'now':<10}  {f}")[:width - 1])
        import textwrap as _tw
        for l in _tw.wrap("PROD = graduating deploys to production · never = grandfathered, only this review shows it",
                          width=width - 1, initial_indent="  ", subsequent_indent="    "):
            print(l)
        # The command goes on its own line, in both avenues: its name is longer
        # in the binary, and a width-driven wrap would then break the two pages
        # apart (ADR 1201).
        print("  a release nobody owns may instead be abandoned:")
        print("    graduate.py abandon <record> --reason \"<why>\"")
        print("  detail: python3 ops/release/graduate.py review --detail <component>")
        return 0
    # --detail: the narrative, one block per item, wrapped to the terminal
    import textwrap
    def para(label, text):
        first = f"    {label:<9}"
        print(textwrap.fill(text, width=width - 1, initial_indent=first, subsequent_indent=" " * len(first)))
    for st, it in rows:
        kind = "fastlane debt" if it["class"] == "hotfix" else "release candidate"
        extra = f" · runnable {it['runnable']}" if it.get("runnable") else ""
        extra += f" · CI red {it['check_red_on']}" if it.get("check_red_on") else (" · grandfathered" if it.get("grandfathered") else "")
        print(f"\n  {st}  {it['component']}  ({kind}, opened {it.get('opened')}, {_age(it, today)}){extra}")
        para("what", it["what"]); para("if left", it["if_left"]); para("if acted", it["if_acted"])
        for b in it.get("blockers", []):
            para("blocked", b)
        para("command", it["command"])
    for it in doc["attention"]:
        print(f"\n  SKEW  {it['component']}")
        para("what", it["what"]); para("if left", it["if_left"]); para("if acted", it["if_acted"]); para("command", it["command"])
    for f in doc["check"]["failures"]:
        print(f"\n  CHECK  {f}")
    print()
    return 0


def cmd_readiness(args):
    doc = build_readiness()
    text = json.dumps(doc, indent=2, sort_keys=False, default=str)
    if args.out:
        pathlib.Path(args.out).write_text(text + "\n")
        print(f"release readiness written: {args.out} "
              f"({len(doc['components'])} components)")
    else:
        print(text)
    return 0



def cmd_abandon(args):
    """Mark a record abandoned WITH a machine-readable reason (C8).

    Exists because the convention it replaces did not survive contact: writing
    the explanation as a YAML comment satisfies a human reader and nothing
    else. This writes fields, so `check`, the console and any other consumer
    can see them."""
    path = pathlib.Path(args.record)
    if not path.is_absolute():
        path = REPO / path
    if not path.exists():
        die(f"no such record: {args.record}")
    rec = load_yaml(path)
    if "_error" in rec:
        die(f"{path} does not parse: {rec['_error']}")
    if rec.get("state") == "graduated":
        die("refusing to abandon a record that already graduated — that is history, not a candidate")
    rec["state"] = "abandoned"
    rec["abandoned_reason"] = args.reason
    if args.superseded_by:
        rec["superseded_by"] = args.superseded_by
    path.write_text(yaml.safe_dump(rec, sort_keys=False))
    rel = path.relative_to(REPO)
    print(f"abandoned: {rel}")
    print(f"  abandoned_reason: {args.reason}")
    if args.superseded_by:
        print(f"  superseded_by:    {args.superseded_by}")
    return 0

def cmd_check(args):
    failures = []
    # C1 — ladder shape
    try:
        ladder, rungs, live = load_ladder()
    except Exception as e:  # noqa: BLE001 — CI wants one line, not a stack
        print(f"C1 FAIL: ladder.yaml unreadable: {e}", file=sys.stderr)
        return 1
    pins = load_pins()
    goatnets = set((pins.get("goatnets") or {}).keys())
    for r in live:
        if r["goatnet"] not in goatnets:
            failures.append(f"C1: live rung {r['name']!r} names goatnet "
                            f"{r['goatnet']!r} not present in fleet-pins.yaml")
    for h in ladder.get("hops", []):
        for g in h.get("gates", []):
            gid = g["id"] if isinstance(g, dict) else g
            if gid not in KNOWN_GATES:
                failures.append(f"C1: unknown gate id {gid!r}")
            if gid == "runtime-health" and isinstance(g, dict):
                frac = g.get("max_memory_fraction", RUNTIME_HEALTH_MAX_MEMORY_FRACTION)
                if isinstance(frac, bool) or not isinstance(frac, (int, float)) or not 0 < frac <= 1:
                    failures.append(f"C1: hop {h.get('from')}->{h.get('to')} runtime-health "
                                    f"max_memory_fraction must be a number in (0, 1]")
                md = g.get("min_days", 1)
                if isinstance(md, bool) or not isinstance(md, int) or md < 1:
                    failures.append(f"C1: hop {h.get('from')}->{h.get('to')} runtime-health "
                                    f"min_days must be a whole number of days >= 1")
    for name, ov in (ladder.get("class_overrides") or {}).items():
        if ov.get("class") not in CLASSES:
            failures.append(f"C1: class_overrides.{name} has unknown class")
        if not ov.get("reason"):
            failures.append(f"C1: class_overrides.{name} missing reason")
    for name, row in (ladder.get("legacy_skew_allowlist") or {}).items():
        if not (row or {}).get("reason"):
            failures.append(f"C1: legacy_skew_allowlist.{name} missing reason")
    # C2 — records
    records = load_records()
    comp_names = set((pins.get("components") or {}).keys())
    for rec in records:
        rp = rec["_path"].relative_to(REPO)
        if "_error" in rec:
            failures.append(f"C2: {rp} unparseable: {rec['_error']}")
            continue
        if rec.get("component") not in comp_names:
            failures.append(f"C2: {rp} names unknown component {rec.get('component')!r}")
        if rec.get("state") not in RECORD_STATES:
            failures.append(f"C2: {rp} has unknown state {rec.get('state')!r}")
        if not DIGEST_RE.search(str(rec.get("candidate_ref"))):
            failures.append(f"C2: {rp} candidate_ref is not digest-pinned")
        if rec.get("class") == "hotfix" and not rec.get("operator_ack"):
            failures.append(f"C2: {rp} hotfix record without operator_ack")
        w = rec.get("soak_waiver")
        if w is not None:
            if not isinstance(w, dict) or not str(w.get("reason") or "").strip():
                failures.append(f"C2: {rp} soak_waiver must be a mapping carrying a "
                                f"non-empty reason — an unexplained waiver is worse "
                                f"than none")
            elif "by" not in w:
                failures.append(f"C2: {rp} soak_waiver names no `by:` — name the "
                                f"operator, or write `by: null` to state that the "
                                f"record cannot attribute it")
        ov = rec.get("runtime_health_override")
        if ov is not None:
            if not isinstance(ov, dict):
                failures.append(f"C2: {rp} runtime_health_override must map a rung to "
                                f"{{by, reason}}")
            else:
                for rung in ov:
                    if not runtime_health_override(rec, rung):
                        failures.append(f"C2: {rp} runtime_health_override.{rung} must name "
                                        f"a non-empty by and reason — an override nobody can "
                                        f"attribute is worse than none")
    # C3/C4 — skew must be declared
    _, live2, _, rows = build_status()
    for row in rows:
        if row["grade"] == "UNDECLARED-SKEW":
            hops = ", ".join(p for p, g in row["pairs"].items() if g == "UNDECLARED-SKEW")
            failures.append(f"C3/C4: {row['component']} cross-rung skew on {hops} has no "
                            f"record and no allowlist row: "
                            f"{ {n: (c.get('value') or c.get('state')) for n, c in row['rungs'].items()} }")
    # C5 — graduated records agree with fleet
    for name in comp_names:
        rec = latest_record(records, name, state="graduated")
        if not rec:
            continue
        newer = [r for r in records if r.get("component") == name and "_error" not in r
                 and str(r["_path"]) > str(rec["_path"])]
        if newer:
            continue
        fleet = (pins["components"][name] or {}).get("fleet")
        if fleet is not None and fleet != rec.get("candidate_ref"):
            failures.append(f"C5: {name} latest graduated record ref differs from "
                            f"fleet value — fold not executed or superseded silently")
    # C6 — stale candidates
    pre, _ = preprod_prod(live)
    for rec in records:
        if rec.get("state") != "candidate" or "_error" in rec:
            continue
        comp_def = pins["components"].get(rec["component"])
        if comp_def is None:
            continue
        target_goatnet = pre["goatnet"] if rec.get("class") != "hotfix" else \
            next(r["goatnet"] for r in live if r["name"] == "prod")
        st, val = effective(comp_def, target_goatnet)
        if st == "value" and val != rec.get("candidate_ref"):
            # A ladder candidate reaches the dev rung before preprod (ADR
            # 1173): while the dev goatnet runs the record's ref, the record
            # is on its way, not stale. The first onboard-runner candidate
            # tripped this rule between the two hops, because
            # the grader (C3/C4) needs the record before the dev deviation
            # and this rule wanted the ref on preprod at the same moment.
            dev = next((r for r in live if r["name"] == "dev"), None)
            if rec.get("class") != "hotfix" and dev is not None:
                dst, dval = effective(comp_def, dev["goatnet"])
                if dst == "value" and dval == rec.get("candidate_ref"):
                    continue
            failures.append(f"C6: {rec['_path'].relative_to(REPO)} is open but "
                            f"{target_goatnet} now runs a different ref — "
                            f"abandon (state: abandoned) or refresh the record")
    # C9 — a hotfix's backfill is owed, not optional. An open hotfix record
    # older than HOTFIX_BACKFILL_MAX_DAYS with no validation evidence fails the
    # tree: the fastlane skipped the soak on the promise the evidence would
    # follow, and a promise nothing enforces is the F-511 class of drift.
    # Filling evidence.validation (the acceptance rows' PASS) and graduating
    # closes it; abandoning with a reason (C8) also closes it.
    today = dt.date.today()
    for rec in records:
        if rec.get("state") != "candidate" or rec.get("class") != "hotfix" or "_error" in rec:
            continue
        try:
            opened = dt.date.fromisoformat(str(rec.get("opened")))
        except ValueError:
            continue
        if opened < HOTFIX_BACKFILL_CUTOFF:
            continue
        age = (today - opened).days
        if age > HOTFIX_BACKFILL_MAX_DAYS and not (rec.get("evidence") or {}).get("validation"):
            failures.append(f"C9: {rec['_path'].relative_to(REPO)} has owed its backfill for {age} days "
                            f"with evidence.validation empty — run the affected live rows, fill the "
                            f"evidence and `graduate`, or abandon it with a reason")
    # C10 — runtime health on a hop `graduate` does not run. The dev->preprod
    # move is a pin edit, so the PR that puts a candidate on the upper rung is
    # where its lower rung's runtime health is held. `records_opened_from` on
    # the gate entry leaves candidates opened before the gate was switched on
    # to `graduate`, which still asks every candidate.
    for h in ladder.get("hops", []):
        if h.get("status") != "live":
            continue
        upper = next((r for r in live if r["name"] == h.get("to")), None)
        if upper is None:
            continue
        for g in h.get("gates", []):
            if gate_id(g) != "runtime-health":
                continue
            since = str((g.get("records_opened_from") if isinstance(g, dict) else None) or "")
            for rec in records:
                if "_error" in rec or rec.get("state") != "candidate" \
                        or (rec.get("class") or "ladder") != "ladder":
                    continue
                if since and str(rec.get("opened") or "") < since:
                    continue
                comp_def = (pins.get("components") or {}).get(rec.get("component"))
                if comp_def is None:
                    continue
                st, val = effective(comp_def, upper["goatnet"])
                if st != "value" or val != rec.get("candidate_ref"):
                    continue
                for f in runtime_health_failures(rec, h, g):
                    failures.append(f"C10: {rec['_path'].relative_to(REPO)} runs on "
                                    f"{upper['goatnet']} but its {h.get('from')}->{h.get('to')} "
                                    f"gate fails: {f}")
    # C7 — graduation sets (ADR 1125). The set is the unit of movement, so a
    # malformed one is a promotion hazard, not a docs nit.
    sets = load_sets()
    seen_open = {}
    for doc in sets:
        rel = doc["_path"].relative_to(REPO)
        if "_error" in doc:
            failures.append(f"C7: {rel} does not parse: {doc['_error']}")
            continue
        if not doc.get("name"):
            failures.append(f"C7: {rel} has no name:")
        if doc.get("state") not in SET_STATES:
            failures.append(f"C7: {rel} state {doc.get('state')!r} is not one of "
                            f"{sorted(SET_STATES)}")
        members = set_members(doc)
        if not members:
            failures.append(f"C7: {rel} names no members — an empty set promotes "
                            f"nothing and hides its own emptiness")
        for m in members:
            comp, ref = m.get("component"), m.get("ref")
            if comp not in pins.get("components", {}):
                failures.append(f"C7: {rel} names unknown component {comp!r}")
                continue
            if not ref or not DIGEST_RE.search(str(ref)):
                failures.append(f"C7: {rel} member {comp} ref is not digest-pinned — "
                                f"build once, promote the digest")
            if latest_record(records, comp, ref=ref) is None:
                failures.append(f"C7: {rel} member {comp} has no record for its exact "
                                f"ref — the set moves refs, the record carries the "
                                f"evidence, and a member with neither is unreviewable")
            if doc.get("state") == "open":
                prev = seen_open.get(comp)
                if prev:
                    failures.append(f"C7: {comp} is in TWO open sets ({prev} and "
                                    f"{rel}) — which one moves it is undecidable")
                seen_open[comp] = rel
    # C8 — an abandoned record must say WHY, in a field a parser can read.
    #
    # "Abandoned, not forgotten" was convention, and convention lost: of 17
    # abandoned records, 8 carry nothing but the original `reason:` (which
    # explains why the candidate was OPENED, not why it was dropped). Several
    # of the rest explain themselves in a YAML COMMENT — fully legible to a
    # human opening the file and completely invisible to yaml.safe_load, so
    # graduate.py, the console, and every surface reading records as data see
    # an abandonment with no reason at all.
    #
    # SCOPED BY OPEN DATE ON PURPOSE. Applying this to all 17 would force
    # someone to reconstruct reasons for 8 historical abandonments, several of
    # them other sessions', where the honest answer is "unknown" — inventing
    # those is exactly the failure this gate exists to prevent. Records opened
    # before the cutoff are left alone; anything opened from the cutoff on must
    # carry the field.
    for rec in records:
        if "_error" in rec or rec.get("state") != "abandoned":
            continue
        opened = str(rec.get("opened") or "")
        if opened < ABANDON_REASON_CUTOFF:
            continue
        rp = rec["_path"].relative_to(REPO)
        if not str(rec.get("abandoned_reason") or "").strip():
            failures.append(
                f"C8: {rp} is abandoned but has no `abandoned_reason:` — a YAML "
                f"comment does not count, because nothing that reads records as "
                f"data can see it. Set it with `graduate.py abandon`.")
        sb = str(rec.get("superseded_by") or "").strip()
        if sb and not (record_dir(rec["component"]) / sb).exists() and "@sha256:" not in sb:
            failures.append(
                f"C8: {rp} names superseded_by {sb!r}, which is neither a record "
                f"file beside it nor a digest-pinned ref")

    if failures:
        print(f"graduation check: {len(failures)} failure(s)", file=sys.stderr)
        for f in failures:
            print(f"  {f}", file=sys.stderr)
        return 1
    print(f"graduation check: OK ({len(rows)} components, "
          f"{len([r for r in records if '_error' not in r])} records, "
          f"{len(live)} live rungs, "
          f"{len([d for d in sets if '_error' not in d])} graduation set(s))")
    return 0


def ack_name(value):
    """--ack names the person on the record, so a placeholder never reaches it."""
    if "<" in value and ">" in value:
        raise argparse.ArgumentTypeError(f"`{value}` is still a placeholder: put your own name, for example --ack \"$(git config user.name)\"")
    return value


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    s = sub.add_parser("status"); s.add_argument("--json", action="store_true")
    s = sub.add_parser("open")
    s.add_argument("component"); s.add_argument("--ref", required=True)
    s.add_argument("--reason", required=True)
    s = sub.add_parser("graduate")
    s.add_argument("component"); s.add_argument("--ack", default=None, type=ack_name)
    s.add_argument("--override-runtime-health", dest="override_runtime_health", default=None,
                   metavar="REASON",
                   help="graduate although the runtime-health gate fails; REASON and the "
                        "--ack name are written into the record")
    s = sub.add_parser("graduate-set")
    s.add_argument("name"); s.add_argument("--ack", default=None, type=ack_name)
    s = sub.add_parser("hotfix")
    s.add_argument("component"); s.add_argument("--ref", required=True)
    s.add_argument("--reason", required=True); s.add_argument("--ack", default=None, type=ack_name)
    # A caller that has ALREADY moved fleet: must say what it moved away from —
    # reading fleet: here would then record the ref being deployed as the ref to
    # roll back to. ops/scripts/fastlane.sh does exactly that: it rewrites
    # fleet-pins.yaml at its step 3 and opens the record at its step 6. Found on
    # the first live fastlane (id-admin-console sha-e1e5358), where
    # previous_fleet_ref came out equal to candidate_ref.
    s.add_argument("--previous-ref", dest="previous_ref", default=None,
                   help="the ref being moved away from; required when fleet: has already been rewritten")
    s = sub.add_parser("review")
    s.add_argument("--json", action="store_true")
    s.add_argument("--ack", default=None, help="your name, filled into the printed graduate commands")
    s.add_argument("--detail", action="store_true", help="the narrative per item: what it is, if left, if acted, command")
    s.add_argument("--no-header", dest="no_header", action="store_true", help="omit the REVIEW line (a wrapper prints its own)")
    s.add_argument("component", nargs="?", default=None, help="limit to one component")
    s = sub.add_parser("readiness")
    s.add_argument("--out", default=None, help="write the document here instead of stdout")
    s = sub.add_parser("abandon")
    s.add_argument("record")
    s.add_argument("--reason", required=True)
    s.add_argument("--superseded-by", dest="superseded_by", default="")
    sub.add_parser("check")
    args = ap.parse_args()
    return {"status": cmd_status, "open": cmd_open, "graduate": cmd_graduate,
            "graduate-set": cmd_graduate_set,
            "hotfix": cmd_hotfix, "review": cmd_review, "readiness": cmd_readiness,
            "abandon": cmd_abandon, "check": cmd_check}[args.cmd](args)


if __name__ == "__main__":
    sys.exit(main())
