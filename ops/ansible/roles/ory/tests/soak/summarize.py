#!/usr/bin/env python3
# Summarize a soak-evidence JSONL into the criterion-check rollup that
# fills `docs/project/idp-decision-review.md` §6 (or any future Ory
# decision-review's equivalent table).
#
# Two modes — auto-detected from the JSONL field set:
#   - single-node    (run-single-node.sh fields):
#       identity_create_http, client_create_ok, token_grant_ok,
#       introspect_ok, kratos_health, hydra_health
#   - 2node-spock    (run-2node-spock.sh fields):
#       create_a_ok, repl_a_to_b, create_b_ok, repl_b_to_a,
#       token_ok, introspect_cross_node, lag_a_bytes, lag_b_bytes,
#       kratos_a_health, kratos_b_health, hydra_a_health, hydra_b_health
#
# Run:
#   python3 ops/ansible/roles/ory/tests/soak/summarize.py \
#     docs/reports/2026-04-25-ory-soak-evidence.jsonl
#
# Optional flags:
#   --markdown   emit a markdown table block ready to paste into the
#                decision-review doc (default).
#   --json       emit a structured JSON summary for downstream tooling.
#
# Pass-rate thresholds (per ADR 1008 default):
#   PASS         every operation green-rate ≥ 99% of ticks
#   CONDITIONAL  every operation ≥ 95% AND no consecutive failure run > 3 ticks
#   FAIL         anything below conditional, or replication lag p95 > 10 MB
#                (2-node only)

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path


SINGLE_NODE_FIELDS = {
    "identity_create_http",
    "client_create_ok",
    "token_grant_ok",
    "introspect_ok",
    "kratos_health",
    "hydra_health",
}

TWONODE_FIELDS = {
    "create_a_ok",
    "repl_a_to_b_db",
    "repl_a_to_b_api",
    "create_b_ok",
    "repl_b_to_a_db",
    "repl_b_to_a_api",
    "token_ok",
    "introspect_cross_node",
    "flow_create_a_ok",
    "flow_repl_a_to_b_db",
    "flow_repl_a_to_b_api",
    "token_a_ok",
    "introspect_a_to_b",
    "concurrent_converged",
    "concurrent_data_loss",
    "f3_app_layer_caught",
    "lag_a_bytes",
    "lag_b_bytes",
    "kratos_a_health",
    "kratos_b_health",
    "hydra_a_health",
    "hydra_b_health",
}

PASS_THRESHOLD = 0.99
CONDITIONAL_THRESHOLD = 0.95
LAG_P95_FAIL_BYTES = 10 * 1024 * 1024  # 10 MB
MAX_CONSECUTIVE_FAILS = 3


def load_ticks(path: Path) -> list[dict]:
    ticks: list[dict] = []
    with path.open() as fh:
        for line_num, line in enumerate(fh, start=1):
            line = line.strip()
            if not line:
                continue
            try:
                ticks.append(json.loads(line))
            except json.JSONDecodeError as e:
                print(
                    f"WARN: line {line_num}: skipping malformed JSON ({e})",
                    file=sys.stderr,
                )
    return ticks


def detect_mode(ticks: list[dict]) -> str:
    if not ticks:
        return "empty"
    keys = set(ticks[0].keys())
    if SINGLE_NODE_FIELDS.issubset(keys):
        return "single-node"
    if TWONODE_FIELDS.issubset(keys):
        return "2node-spock"
    return "unknown"


def green_rate(ticks: list[dict], field: str, ok_fn) -> tuple[float, int]:
    """Return (rate in [0,1], max consecutive failure run)."""
    total = 0
    green = 0
    max_run = 0
    cur_run = 0
    for t in ticks:
        if field not in t:
            continue
        total += 1
        if ok_fn(t[field]):
            green += 1
            cur_run = 0
        else:
            cur_run += 1
            max_run = max(max_run, cur_run)
    return ((green / total) if total else 0.0, max_run)


def lag_p95(ticks: list[dict], field: str) -> int:
    vals = [int(t[field]) for t in ticks if field in t]
    if not vals:
        return 0
    if len(vals) < 20:
        return max(vals)
    sorted_vals = sorted(vals)
    idx = max(0, int(0.95 * len(sorted_vals)) - 1)
    return sorted_vals[idx]


def verdict(rate: float, max_run: int) -> str:
    if rate >= PASS_THRESHOLD and max_run <= MAX_CONSECUTIVE_FAILS:
        return "PASS"
    if rate >= CONDITIONAL_THRESHOLD and max_run <= MAX_CONSECUTIVE_FAILS * 2:
        return "CONDITIONAL"
    return "FAIL"


def summarize_single_node(ticks: list[dict]) -> dict:
    n = len(ticks)
    elapsed = ticks[-1].get("elapsed_s", 0) if ticks else 0
    rows = []

    def add(label, field, ok_fn):
        rate, max_run = green_rate(ticks, field, ok_fn)
        rows.append(
            {
                "criterion": label,
                "field": field,
                "rate": rate,
                "max_consecutive_fail": max_run,
                "verdict": verdict(rate, max_run),
            }
        )

    add("Identity create (Kratos admin API returns 201)", "identity_create_http", lambda v: int(v) == 201)
    add("OAuth2 client create (Hydra admin API returns full client)", "client_create_ok", lambda v: int(v) == 1)
    add("client_credentials token grant", "token_grant_ok", lambda v: int(v) == 1)
    add("Token introspect (active=true)", "introspect_ok", lambda v: int(v) == 1)
    add("Kratos /health/ready (200)", "kratos_health", lambda v: int(v) == 200)
    add("Hydra /health/ready (200)", "hydra_health", lambda v: int(v) == 200)

    overall = "PASS"
    for r in rows:
        if r["verdict"] == "FAIL":
            overall = "FAIL"
            break
        if r["verdict"] == "CONDITIONAL" and overall != "FAIL":
            overall = "CONDITIONAL"

    return {
        "mode": "single-node",
        "ticks": n,
        "elapsed_s": elapsed,
        "rows": rows,
        "overall_verdict": overall,
    }


def summarize_twonode(ticks: list[dict]) -> dict:
    n = len(ticks)
    elapsed = ticks[-1].get("elapsed_s", 0) if ticks else 0
    rows = []

    def add(label, field, ok_fn):
        rate, max_run = green_rate(ticks, field, ok_fn)
        rows.append(
            {
                "criterion": label,
                "field": field,
                "rate": rate,
                "max_consecutive_fail": max_run,
                "verdict": verdict(rate, max_run),
            }
        )

    add("Identity create at A (HTTP 201 + ID returned)", "create_a_ok", lambda v: int(v) == 1)
    add("Spock A→B: identity row reaches postgres-b (DB layer)", "repl_a_to_b_db", lambda v: int(v) == 1)
    add("Kratos-B API resolves identity from A (kratos multi-tenancy + Spock both work)", "repl_a_to_b_api", lambda v: int(v) == 1)
    add("OAuth2 client create at B", "create_b_ok", lambda v: int(v) == 1)
    add("Spock B→A: hydra_client row reaches postgres-a (DB layer)", "repl_b_to_a_db", lambda v: int(v) == 1)
    add("Hydra-A API resolves client from B (hydra multi-tenancy + Spock both work)", "repl_b_to_a_api", lambda v: int(v) == 1)
    add("Token grant via B", "token_ok", lambda v: int(v) == 1)
    add("Cross-node introspect (token from B, validated at A)", "introspect_cross_node", lambda v: int(v) == 1)
    # A14: login flow A→B race
    add("A14 — Login flow created at A (HTTP 200 + flow id)", "flow_create_a_ok", lambda v: int(v) == 1)
    add("A14 — Spock A→B: login flow row reaches postgres-b (DB layer)", "flow_repl_a_to_b_db", lambda v: int(v) == 1)
    add("A14 — Kratos-B API resolves login flow from A (sticky-routing risk if FAIL)", "flow_repl_a_to_b_api", lambda v: int(v) == 1)
    # A15: token forward direction
    add("A15 — Token grant via A", "token_a_ok", lambda v: int(v) == 1)
    add("A15 — Cross-node introspect forward (token from A, validated at B)", "introspect_a_to_b", lambda v: int(v) == 1)
    add("Kratos-A /health/ready", "kratos_a_health", lambda v: int(v) == 200)
    add("Kratos-B /health/ready", "kratos_b_health", lambda v: int(v) == 200)
    add("Hydra-A /health/ready", "hydra_a_health", lambda v: int(v) == 200)
    add("Hydra-B /health/ready", "hydra_b_health", lambda v: int(v) == 200)

    # F3: only an observed 201 + 409 counts as a duplicate rejection.
    # Historical ticks counted HTTP 200, discarded statuses, and cannot
    # distinguish concurrent acceptance from a request/measurement failure.
    add(
        "F3 — concurrent create returned one identity and one conflict at A+B",
        "f3_app_layer_caught",
        lambda v: int(v) == 1,
    )

    # A16: concurrent-mutation LWW probe — INVERTED interpretation.
    # data_loss == 1 means the probe observed last-write-wins drop one
    # update (which is what we EXPECT under Spock's default conflict
    # resolution; a 0 here would mean the probe didn't run, e.g. because
    # repl_a_to_b_db was 0 and the test was skipped).
    # Verdict here is informational, not pass/fail — we report the rate
    # at which the loss was observed so the decision review can size the
    # production residual risk + decide between sticky write-affinity vs
    # accepting LWW.
    loss_observed_rate, _ = green_rate(ticks, "concurrent_data_loss", lambda v: int(v) == 1)
    converged_rate, _ = green_rate(ticks, "concurrent_converged", lambda v: int(v) == 1)
    rows.append(
        {
            "criterion": "A16 — Concurrent A+B mutation observed LWW data loss (informational; expect ≈100% — confirms Spock LWW resolution + production data-loss risk surface area)",
            "field": "concurrent_data_loss",
            "rate": loss_observed_rate,
            "max_consecutive_fail": 0,
            "verdict": "INFO",
            "extra": {
                "loss_observed_rate": loss_observed_rate,
                "converged_rate": converged_rate,
            },
        }
    )

    lag_a = lag_p95(ticks, "lag_a_bytes")
    lag_b = lag_p95(ticks, "lag_b_bytes")
    lag_verdict = (
        "FAIL"
        if max(lag_a, lag_b) > LAG_P95_FAIL_BYTES
        else "PASS"
    )
    rows.append(
        {
            "criterion": "Spock replication lag p95",
            "field": "lag_a_bytes/lag_b_bytes (max)",
            "rate": float(max(lag_a, lag_b)),
            "max_consecutive_fail": 0,
            "verdict": lag_verdict,
            "extra": {
                "lag_a_p95_bytes": lag_a,
                "lag_b_p95_bytes": lag_b,
                "fail_threshold_bytes": LAG_P95_FAIL_BYTES,
            },
        }
    )

    # Nemesis events (ADR 1010 D6 partial F1+F2 coverage) — informational
    # rollup when the JSONL contains nemesis_event tags. The event tick
    # itself is expected to show degraded green-rate (that's the point);
    # the recovery tick is the one we care about — if nemesis_in_recovery
    # ticks have failure rates indistinguishable from steady-state, the
    # discipline (D2 lifespan budget + D3 retry + restart-unless-stopped)
    # held. If recovery ticks show elevated failure, the operator should
    # widen the lifespan budget or extend the retry policy.
    nemesis_ticks = [t for t in ticks if t.get("nemesis_event")]
    recovery_ticks = [t for t in ticks if t.get("nemesis_in_recovery") == 1]
    if nemesis_ticks or recovery_ticks:
        from collections import Counter

        event_counts = Counter(t.get("nemesis_event", "") for t in nemesis_ticks)

        # Repl-catchup rate on recovery ticks (the load-bearing question:
        # did Spock + kratos restart back to steady-state on the next tick?).
        recovery_repl_ok = sum(
            1 for t in recovery_ticks if int(t.get("repl_a_to_b_db", 0)) == 1
        )
        recovery_repl_total = len(recovery_ticks)
        recovery_rate = (
            recovery_repl_ok / recovery_repl_total if recovery_repl_total else 0.0
        )
        # Use the same threshold shape as steady-state but acknowledge
        # one-cycle catchup is acceptable; a single failed recovery tick
        # is informational, not pass/fail.
        rec_verdict = "INFO"
        if recovery_repl_total >= 5 and recovery_rate < CONDITIONAL_THRESHOLD:
            rec_verdict = "FAIL"
        rows.append(
            {
                "criterion": "Nemesis recovery — repl_a_to_b_db green on first post-event tick (informational unless ≥5 events fired and recovery <95%)",
                "field": "nemesis_in_recovery + repl_a_to_b_db",
                "rate": recovery_rate,
                "max_consecutive_fail": 0,
                "verdict": rec_verdict,
                "extra": {
                    "events_fired": dict(event_counts),
                    "recovery_ticks": recovery_repl_total,
                    "recovery_repl_ok": recovery_repl_ok,
                },
            }
        )

    overall = "PASS"
    for r in rows:
        if r["verdict"] == "INFO":
            continue
        if r["verdict"] == "FAIL":
            overall = "FAIL"
            break
        if r["verdict"] == "CONDITIONAL" and overall != "FAIL":
            overall = "CONDITIONAL"

    return {
        "mode": "2node-spock",
        "ticks": n,
        "elapsed_s": elapsed,
        "rows": rows,
        "overall_verdict": overall,
    }


def render_markdown(summary: dict, src_path: Path) -> str:
    out = []
    out.append(f"# Soak summary — {src_path.name}\n")
    out.append(f"- Mode: **{summary['mode']}**")
    out.append(f"- Ticks: **{summary['ticks']}**")
    out.append(f"- Elapsed: **{summary['elapsed_s']}s**")
    out.append(f"- Overall verdict: **{summary['overall_verdict']}**\n")
    out.append("| Criterion | Green-rate | Max consecutive fail | Verdict |")
    out.append("|---|---|---|---|")
    nemesis_extra = None
    for r in summary["rows"]:
        if r["field"].endswith("(max)"):
            rate_str = f"{int(r['rate'])} bytes"
        elif r["field"] == "nemesis_in_recovery + repl_a_to_b_db":
            extra = r.get("extra", {})
            rate_str = f"{r['rate'] * 100:.2f}% of {extra.get('recovery_ticks', 0)} recovery ticks"
            nemesis_extra = extra
        elif r["verdict"] == "INFO":
            rate_str = f"observed in {r['rate'] * 100:.2f}% of ticks"
        else:
            rate_str = f"{r['rate'] * 100:.2f}%"
        out.append(
            f"| {r['criterion']} | {rate_str} | {r['max_consecutive_fail']} | {r['verdict']} |"
        )
    if nemesis_extra and nemesis_extra.get("events_fired"):
        out.append("")
        out.append("**Nemesis events fired:**")
        for evt, n in nemesis_extra["events_fired"].items():
            out.append(f"- `{evt}` × {n}")
    out.append("")
    out.append("Thresholds: PASS = green-rate ≥ 99% AND max consecutive fails ≤ 3.")
    out.append(
        "CONDITIONAL = green-rate ≥ 95% AND max consecutive fails ≤ 6. "
        "Otherwise FAIL."
    )
    if summary["mode"] == "2node-spock":
        out.append(
            "Replication lag: PASS if both p95 lag values ≤ 10 MB; FAIL above."
        )
    return "\n".join(out) + "\n"


def main():
    p = argparse.ArgumentParser(description="Summarize an Ory soak JSONL")
    p.add_argument("evidence_path", help="Path to soak-evidence JSONL")
    p.add_argument("--json", action="store_true", help="Emit JSON instead of markdown")
    args = p.parse_args()

    src = Path(args.evidence_path)
    if not src.exists():
        print(f"FATAL: {src} does not exist", file=sys.stderr)
        sys.exit(2)

    ticks = load_ticks(src)
    if not ticks:
        print(f"FATAL: {src} has no parseable ticks", file=sys.stderr)
        sys.exit(2)

    mode = detect_mode(ticks)
    if mode == "single-node":
        summary = summarize_single_node(ticks)
    elif mode == "2node-spock":
        summary = summarize_twonode(ticks)
    else:
        print(f"FATAL: unknown JSONL shape (mode={mode})", file=sys.stderr)
        sys.exit(2)

    if args.json:
        json.dump(summary, sys.stdout, indent=2)
        print()
    else:
        print(render_markdown(summary, src))


if __name__ == "__main__":
    main()
