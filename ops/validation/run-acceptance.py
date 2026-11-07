#!/usr/bin/env python3
"""run-acceptance.py — the goatnet e2e acceptance-suite runner (ADR 1115).

Reads ops/validation/acceptance-tests.yaml, resolves each row's params against
a per-env value set, computes which rows are ELIGIBLE (their `requires`
components are in the deployed set — from the ADR 1060 deploy ledger), runs the
eligible ones through the probe library, and prints an honesty-graded summary.

Usage:
  run-acceptance.py --env site-a [--values <file>] [--deployed a,b,c | --ledger <jsonl>]
                    [--only <id>] [--var k=v ...] [--json <out.jsonl>]
  run-acceptance.py --self-test          # offline; proves each core probe RED then GREEN

Honesty grammar (never fudged): PASS | FAIL | UNKNOWN | SKIPPED(dep) | N/A.
The gate counts ONLY PASS. UNKNOWN never counts as green — a probe that cannot
run is not a probe that passed (the runbook's own "a gate that cannot run is
not a gate that passed", made mechanical).

Exit: 0 iff every eligible, applicable row is PASS. Non-zero otherwise, so it
wires as a standup phase-13 gate and as CI.
"""
from __future__ import annotations

import argparse
import contextlib
import io
import json
import os
import re
import shutil
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import probes  # noqa: E402

try:
    import yaml
except ImportError:
    sys.exit("run-acceptance: needs PyYAML (pip3 install pyyaml)")

MANIFEST = HERE / "acceptance-tests.yaml"
_VAR = re.compile(r"\{\{\s*([a-zA-Z0-9_]+)\s*\}\}")


def resolve(obj, values):
    """Substitute {{ var }} from `values`. Unresolved -> sentinel so the probe
    reports UNKNOWN rather than silently probing a literal '{{ mesh_ip_a }}'."""
    if isinstance(obj, str):
        # A few cmd templates emit a literal Go template ({{.Names}}) via the
        # jinja idiom {{ '{{' }} … {{ '}}' }}. Translate that to real braces
        # first; _VAR below won't match {{.Names}} because the dot isn't \w, so
        # it survives to the shell intact. Without this the brace idiom reached
        # docker verbatim and every such row FAILed on "failed to parse template".
        obj = obj.replace("{{ '{{' }}", "{{").replace("{{ '}}' }}", "}}")
        missing = []

        def sub(m):
            k = m.group(1)
            if k in values:
                return str(values[k])
            missing.append(k)
            return f"<UNRESOLVED:{k}>"

        out = _VAR.sub(sub, obj)
        return out
    if isinstance(obj, dict):
        return {k: resolve(v, values) for k, v in obj.items()}
    if isinstance(obj, list):
        return [resolve(v, values) for v in obj]
    return obj


def load_values(env, values_file, overrides):
    # `env` is the goatnet as the runner was asked for it; rows that hand it
    # to a script (`--env {{ env }}`) read it from here.
    values = {"site_id": env, "env": env}
    if values_file and Path(values_file).is_file():
        values.update(yaml.safe_load(Path(values_file).read_text()) or {})
    for kv in overrides or []:
        if "=" in kv:
            k, v = kv.split("=", 1)
            values[k] = v
    # The goat CLI rows that read the fabric run as probe_context on probe_via:
    # the goatnet's non-person assay probe where it declares one
    # (generate-values.py), else the operator's own goat_context/goat_via. A
    # --var goat_context/goat_via given on this run wins over a values-file
    # probe, so a run pointed at a context reads with that context.
    given = {kv.split("=", 1)[0] for kv in (overrides or []) if "=" in kv}
    for probe, own in (("probe_context", "goat_context"), ("probe_via", "goat_via")):
        if own in given and probe not in given:
            values[probe] = values.get(own)
        values.setdefault(probe, values.get(own))
    # allow env-var backfill for tokens referenced as auth_header etc.
    return values


def _truthy(v):
    if isinstance(v, bool):
        return v
    if v is None:
        return False
    if isinstance(v, str):
        return v.strip().lower() not in ("", "false", "no", "0")
    return bool(v)


def _matches(got, want):
    # a bool want compares truthiness; anything else compares normalized equality
    if isinstance(want, bool):
        return _truthy(got) == want
    return str(got).strip().lower() == str(want).strip().lower()


def truthy_applies(expr, values):
    """applies_when: 'always'|None|True -> always; a var NAME (str) -> that var is
    truthy in values; a MAPPING {var: expected, ...} -> applies iff every var
    matches its expected value (a bool expected compares truthiness). The mapping
    form is what the operator-ssh-login rows use ({intra_site_pair: true} /
    {has_obs_host: true}); before this it crashed the runner with 'unhashable
    type: dict' and no phase-13 gate could run on a site that carried them."""
    if expr in (None, "always", True):
        return True
    if isinstance(expr, dict):
        return all(_matches(values.get(k), v) for k, v in expr.items())
    val = values.get(expr)
    if isinstance(val, str):
        return val.lower() not in ("", "false", "no", "0")
    return bool(val)


def run(argv=None):
    ap = argparse.ArgumentParser()
    ap.add_argument("--env")
    ap.add_argument("--manifest", default=str(MANIFEST))
    ap.add_argument("--values")
    ap.add_argument("--deployed", help="comma-separated deployed component names")
    ap.add_argument("--ledger", help="deploy-history jsonl; components read from it")
    ap.add_argument("--endstate", help="the site's derived end state (ops/goatbuilder/endstate/endstate.py "
                    "model <site>): a row whose `requires` names only services the site declares "
                    "absent reads N/A (ADR 1206)")
    ap.add_argument("--only", help="run only these row ids (comma-separated)")
    ap.add_argument("--all-eligible", action="store_true",
                    help="run every applicable row (the default when no --only "
                    "is given); explicit for the goat-standup phase-13 gate")
    ap.add_argument("--var", action="append", default=[])
    ap.add_argument("--json", dest="json_out")
    ap.add_argument("--by-layer", action="store_true",
                    help="after the rows, a table of verdicts per layer (the first "
                    "segment of each row's aspect)")
    ap.add_argument("--plan", action="store_true",
                    help="probe nothing: list each row this goatnet's values make "
                    "applicable, by layer, and the rows that read N/A")
    ap.add_argument("--require-pass", action="store_true",
                    help="exit 2 (untested) unless every selected row reached PASS; "
                         "UNKNOWN or SKIPPED rows are rows that did not run. The "
                         "release drivers pass this: a candidate is never promoted "
                         "on rows that could not run")
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args(argv)

    if args.self_test:
        return self_test()

    if not args.env:
        ap.error("--env required (or --self-test)")

    manifest = yaml.safe_load(Path(args.manifest).read_text())
    tests = manifest.get("tests", [])
    values = load_values(args.env, args.values, args.var)

    deployed = set()
    if args.deployed:
        deployed = {c.strip() for c in args.deployed.split(",") if c.strip()}
    elif args.ledger and Path(args.ledger).is_file():
        for line in Path(args.ledger).read_text().splitlines():
            try:
                deployed.add(json.loads(line).get("component"))
            except json.JSONDecodeError:
                pass  # skip malformed ledger line
        deployed.discard(None)

    absent = declared_absent(args.endstate)

    if args.plan:
        return plan(args, tests, values, absent)

    results = []
    # TWO PASSES, POSITIVES FIRST. A `must_refuse` row asserts that something is
    # DENIED, and a denial is indistinguishable from a path that was never live:
    # run it against a broken deployment and it passes vacuously, reporting a
    # boundary that is actually just an outage. So every negative names the
    # positive that proves the path was carrying, and is only counted when that
    # positive PASSED (docs/design/staged-delivery-pattern.md, acceptance tier 3).
    positives = [t for t in tests if not t.get("must_refuse")]
    negatives = [t for t in tests if t.get("must_refuse")]

    # --only names rows that must exist. An id that matches nothing would run
    # nothing and, previously print "GATE: PASS" over zero rows — the
    # nothing-heard-is-green failure this runner exists to refuse.
    only_ids = {x.strip() for x in (args.only or "").split(",") if x.strip()}
    if only_ids:
        known = {t.get("id") for t in tests}
        if missing := sorted(only_ids - known):
            sys.exit(f"--only names row id(s) not in the manifest: {', '.join(missing)} "
                     f"— a row that does not exist cannot pass")

    for t in positives + negatives:
        if only_ids and t.get("id") not in only_ids:
            continue
        vid, probe_type = t["id"], t.get("probe")

        # A negative whose paired positive did not pass proves nothing. Say so
        # rather than counting it — this is the difference between a boundary
        # that holds and a link that is down.
        if t.get("must_refuse"):
            pair = t.get("paired_positive")
            if not pair:
                results.append((vid, probes.Verdict(
                    probes.UNKNOWN,
                    "must_refuse row names no paired_positive; a refusal with nothing "
                    "proving the path was live cannot be counted", probe_type), t))
                continue
            pv = next((v for (i, v, _) in results if i == pair), None)
            if pv is None:
                results.append((vid, probes.Verdict(
                    probes.UNKNOWN, f"paired positive '{pair}' was not run", probe_type), t))
                continue
            if pv.status != probes.PASS:
                results.append((vid, probes.Verdict(
                    probes.SKIPPED,
                    f"paired positive '{pair}' is {pv.status} — a refusal asserted "
                    f"against a path that is not carrying passes vacuously", probe_type), t))
                continue
        # applicability: the site's declared end state first (ADR 1206), then the row's own predicate
        gone = sorted(set(t.get("requires", [])) & absent)
        if gone:
            results.append((vid, probes.Verdict(probes.NA, f"declared absent on this site: {', '.join(gone)}", probe_type), t))
            continue
        if not truthy_applies(t.get("applies_when"), values):
            results.append((vid, probes.Verdict(probes.NA, "applies_when false", probe_type), t))
            continue
        # eligibility (deploy-ledger gated); if no deployed set given, attempt anyway
        req = set(t.get("requires", []))
        if deployed and not req.issubset(deployed):
            missing = ", ".join(sorted(req - deployed))
            results.append((vid, probes.Verdict(probes.SKIPPED, f"deps unmet: {missing}", probe_type), t))
            continue
        fn = probes.REGISTRY.get(probe_type)
        if not fn:
            results.append((vid, probes.Verdict(probes.UNKNOWN, f"no probe type '{probe_type}'", probe_type), t))
            continue
        params = resolve(t.get("params", {}), values)
        # inject a token from env if the row names auth: <ENVVAR>
        if "auth" in params:
            tok = os.environ.get(str(params["auth"]))
            if not tok:
                # Without the token the call is unauthenticated and a 401 says
                # nothing about the goatnet: the probe cannot answer.
                results.append((vid, probes.Verdict(
                    probes.UNKNOWN, f"needs ${params['auth']} in the environment "
                    "(run under ops/scripts/with-secrets)", probe_type), t))
                continue
            params["auth_header"] = f"Authorization: Bearer {tok}"
        # unresolved var guard
        if any("<UNRESOLVED:" in str(v) for v in _flatten(params)):
            un = [str(v) for v in _flatten(params) if "<UNRESOLVED:" in str(v)]
            results.append((vid, probes.Verdict(probes.UNKNOWN, f"unresolved vars: {un}", probe_type), t))
            continue
        try:
            v = run_one(fn, probe_type, params, t, values)
        except Exception as e:  # noqa: BLE001
            v = probes.Verdict(probes.UNKNOWN, f"probe raised: {e}", probe_type)
        if t.get("must_refuse"):
            v = invert_refusal(v, probe_type)
        results.append((vid, v, t))

    return report(args, results)


def declared_absent(path):
    """The acceptance tokens owned only by services the site's end state declares absent."""
    if not path:
        return set()
    m = json.loads(Path(path).read_text())
    gone, kept = set(), set()
    for s in (m.get("services") or {}).values():
        (gone if s.get("present") is False else kept).update(s.get("assay") or [])
    return gone - kept


def layer_of(t):
    """The layer a row belongs to: its aspect's first segment (host, mesh,
    fabric, identity, federation, deploy …)."""
    return (t.get("aspect") or "other").split("/", 1)[0]


def print_by_layer(results):
    order = ["PASS", "FAIL", "UNKNOWN", "SKIPPED", "N/A"]
    table = {}
    for _, v, t in results:
        row = table.setdefault(layer_of(t), dict.fromkeys(order, 0))
        row[v.status] = row.get(v.status, 0) + 1
    print("\nby layer:        " + "  ".join(f"{k:>7}" for k in order))
    for layer in sorted(table):
        print(f"  {layer:15s} " + "  ".join(f"{table[layer][k]:>7}" for k in order))


def plan(args, tests, values, absent=frozenset()):
    """--plan: what the assay would run on this goatnet, by layer, probing
    nothing. Deterministic for a given value set, so a rehearsal can keep it
    as a checkpoint. A row whose `requires` names a service the end state
    declares absent is N/A here too, as it is when the rows run (ADR 1206)."""
    applicable, na = {}, []
    for t in tests:
        if set(t.get("requires", [])) & absent:
            na.append(t["id"])
        elif truthy_applies(t.get("applies_when"), values):
            applicable.setdefault(layer_of(t), []).append(t)
        else:
            na.append(t["id"])
    total = sum(len(v) for v in applicable.values())
    print(f"assay plan: env={args.env}  {total} applicable rows, {len(na)} N/A")
    for layer in sorted(applicable):
        rows = applicable[layer]
        print(f"\n  {layer} ({len(rows)})")
        for t in rows:
            kind = "refuse" if t.get("must_refuse") else "prove "
            print(f"    {kind} {t['id']:44s} {t.get('probe', '')}")
    if na:
        print(f"\n  N/A here: {', '.join(sorted(na))}")
    return 0


def invert_refusal(v, probe_type):
    """INVERT a must_refuse row. The probe answering normally means the thing
    we require to be refused was ALLOWED, which is the finding. UNKNOWN stays
    UNKNOWN either way: a probe that could not run tells us nothing about
    whether the boundary holds."""
    if v.status == probes.PASS:
        return probes.Verdict(
            probes.FAIL,
            f"ALLOWED, and this row requires a refusal — {v.evidence}", probe_type)
    if v.status == probes.FAIL:
        return probes.Verdict(probes.PASS, f"refused, as required — {v.evidence}", probe_type)
    return v


def run_one(fn, probe_type, params, t, values):
    """Dispatch a probe, expanding a batch row (names_from / hosts_from /
    for_each) into N sub-probes and aggregating: any FAIL -> FAIL; else any
    UNKNOWN -> UNKNOWN; else PASS. The list comes from `values` (e.g.
    isitup_services, relay_public_ips); an absent/empty list is UNKNOWN, not a
    silent pass."""
    batch_key = next((k for k in ("names_from", "hosts_from", "for_each") if k in params), None)
    if not batch_key:
        return fn(params)
    src = params.get(batch_key)
    items = values.get(src)
    if not items:
        return probes.Verdict(probes.UNKNOWN, f"{batch_key}={src} resolved no list "
                              "in the value set (provide it via --values)", probe_type)
    if isinstance(items, dict):
        items = list(items.keys())
    base = {k: v for k, v in params.items() if k not in ("names_from", "hosts_from", "for_each", "cmd_template")}
    fails, unknowns, n = [], 0, 0
    for it in items:
        n += 1
        sub = dict(base)
        if batch_key == "names_from":
            sub["name"] = it
        elif batch_key == "hosts_from":
            sub["host"] = it
        else:  # for_each with a cmd_template
            tmpl = params.get("cmd_template", "")
            sub["cmd"] = tmpl.replace("{item}", str(it)).replace("{svc}", str(it))
        r = fn(sub)
        if r.status == probes.FAIL:
            fails.append(f"{it}: {r.evidence[:50]}")
        elif r.status in (probes.UNKNOWN,):
            unknowns += 1
    if fails:
        return probes.Verdict(probes.FAIL, f"{len(fails)}/{n} failed — " + "; ".join(fails[:4]), probe_type)
    if unknowns:
        return probes.Verdict(probes.UNKNOWN, f"{unknowns}/{n} indeterminate (rest ok)", probe_type)
    return probes.Verdict(probes.PASS, f"all {n} ok", probe_type)


def _flatten(obj):
    if isinstance(obj, dict):
        for v in obj.values():
            yield from _flatten(v)
    elif isinstance(obj, list):
        for v in obj:
            yield from _flatten(v)
    else:
        yield obj


def report(args, results):
    counts = {}
    for _, v, _ in results:
        counts[v.status] = counts.get(v.status, 0) + 1
    print(f"acceptance: env={args.env}  " + "  ".join(f"{k}={counts[k]}" for k in sorted(counts)))
    for vid, v, t in results:
        mark = {"PASS": "PASS ", "FAIL": "FAIL ", "UNKNOWN": "UNK  ",
                "SKIPPED": "SKIP ", "N/A": "N/A  "}.get(v.status, v.status)
        line = f"  {mark} {vid:38s} {v.evidence[:90]}"
        if v.status == "FAIL" and t.get("caught_would_have"):
            line += f"\n         ↳ would have caught: {t['caught_would_have']}"
        print(line)
    if args.json_out:
        with open(args.json_out, "w") as fh:
            for vid, v, t in results:
                fh.write(json.dumps({"id": vid, "status": v.status,
                                     "evidence": v.evidence, "probe": v.probe,
                                     "layer": layer_of(t), "aspect": t.get("aspect", ""),
                                     "requires": t.get("requires", [])}) + "\n")
    if getattr(args, "by_layer", False):
        print_by_layer(results)
    # gate: any FAIL fails; UNKNOWN on an eligible row is NOT green either, but
    # we exit-fail only on FAIL so unreachable-from-CI runs don't red-flag the
    # gate spuriously — the operator's live run treats UNKNOWN as must-resolve.
    fails = counts.get("FAIL", 0)
    passes = counts.get("PASS", 0)
    if fails == 0 and passes == 0:
        # Nothing ran, or nothing that ran could answer: not a pass. The gate
        # counts only PASS (ADR 1115 decision 3).
        print("\nGATE: NOTHING PASSED — no row reached PASS"
              + (f"; {counts['UNKNOWN']} UNKNOWN" if counts.get("UNKNOWN") else "")
              + (f"; {counts['SKIPPED']} SKIPPED" if counts.get("SKIPPED") else "")
              + (f"; {counts['N/A']} N/A" if counts.get("N/A") else "")
              + ("; 0 rows selected" if not results else ""))
        return 2
    not_run = counts.get("UNKNOWN", 0) + counts.get("SKIPPED", 0)
    if getattr(args, "require_pass", False) and fails == 0 and not_run:
        # A release step must not promote on rows that did not run: the gate
        # counts only PASS (ADR 1115 decision 3), so this is untested, never green.
        print(f"\nGATE: UNTESTED — {passes} PASS, {not_run} row(s) did not run"
              + (f" ({counts['UNKNOWN']} UNKNOWN)" if counts.get("UNKNOWN") else "")
              + (f" ({counts['SKIPPED']} SKIPPED)" if counts.get("SKIPPED") else ""))
        return 2
    print(f"\nGATE: {'PASS — no failures' if fails == 0 else f'FAIL — {fails} failing row(s)'}"
          + (f"; {counts['UNKNOWN']} UNKNOWN to resolve on a live mesh run" if counts.get("UNKNOWN") else ""))
    return 1 if fails else 0


# --------------------------------------------------------------------------
# --self-test: prove each core probe goes RED on a known-broken fixture and
# GREEN on a known-good one. A probe trusted before it's proven red is worse
# than no probe (friction row 42 — the checker that greened on its own excuse).
# --------------------------------------------------------------------------
def self_test():
    ok = True

    def check(name, got, want):
        nonlocal ok
        good = got == want
        ok = ok and good
        print(f"  {'ok  ' if good else 'FAIL'} {name}: got {got}, want {want}")

    with tempfile.TemporaryDirectory() as d:
        # dns_resolve: localhost resolves; a bogus name fails
        check("dns green", probes.dns_resolve({"name": "localhost"}).status, probes.PASS)
        check("dns RED", probes.dns_resolve({"name": "no.such.host.invalid."}).status, probes.FAIL)
        # cmd_assert: true passes with expect; a wrong expect fails
        check("cmd green", probes.cmd_assert({"cmd": "echo zagros-prod", "expect": "zagros-prod"}).status, probes.PASS)
        check("cmd RED", probes.cmd_assert({"cmd": "echo zagros-prod", "expect": "SQL_ASCII"}).status, probes.FAIL)
        check("cmd rc RED", probes.cmd_assert({"cmd": "exit 3"}).status, probes.FAIL)
        # a door (rc.sh GOAT_ROWS_SSH_JUMP) adds a tunnel hop to host ssh, and only then;
        # the tunnel hop skips host-key checking (a door port's key churns), the
        # final hop does not
        _saved = os.environ.pop("GOAT_ROWS_SSH_JUMP", None)
        check("no door, no jump", probes._ssh_jump(), "")
        os.environ["GOAT_ROWS_SSH_JUMP"] = "goat@127.0.0.1:12201"
        _j = probes._ssh_jump()
        check("door jump through the door port", ("-p 12201 -W %h:%p goat@127.0.0.1" in _j, _j.startswith("-o ProxyCommand=")), (True, True))
        check("door hop skips host-key checking", "StrictHostKeyChecking=no" in _j, True)
        os.environ.pop("GOAT_ROWS_SSH_JUMP")
        if _saved is not None:
            os.environ["GOAT_ROWS_SSH_JUMP"] = _saved
        # tls_identity against a generated self-signed PEM
        pem = Path(d) / "t.pem"
        key = Path(d) / "t.key"
        import subprocess
        subprocess.run(["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes",
                        "-keyout", str(key), "-out", str(pem), "-days", "1",
                        "-subj", "/O=goat/OU=zagros-prod/CN=goat-zenoh-router",
                        "-addext", "subjectAltName=IP:100.127.0.217"],
                       capture_output=True)
        if pem.exists():
            check("tls green", probes.tls_identity({"pem_file": str(pem),
                  "expect_cn": "goat-zenoh-router", "expect_san_ip": "100.127.0.217"}).status, probes.PASS)
            check("tls RED (stale IP)", probes.tls_identity({"pem_file": str(pem),
                  "expect_san_ip": "100.127.0.110"}).status, probes.FAIL)
            check("tls RED (refuse default)", probes.tls_identity({"pem_file": str(pem),
                  "refuse_cn": "goat-zenoh-router"}).status, probes.FAIL)
        else:
            print("  skip tls: openssl unavailable")
        # probe identity: the fabric rows' probe_* fall back to the operator's
        # goat_*, a declared probe wins over that, and a --var goat_* on the
        # run wins over a declared probe.
        v0 = load_values("x", None, ["goat_context=me", "goat_via=localhost"])
        check("probe falls back to goat_*", (v0["probe_context"], v0["probe_via"]), ("me", "localhost"))
        with tempfile.NamedTemporaryFile("w", suffix=".yaml", delete=False) as vf:
            vf.write('goat_context: "x"\ngoat_via: "localhost"\nprobe_context: "assay-probe"\nprobe_via: "u@h"\n')
        v1 = load_values("x", vf.name, [])
        check("declared probe used", (v1["probe_context"], v1["probe_via"]), ("assay-probe", "u@h"))
        v2 = load_values("x", vf.name, ["goat_context=mine", "goat_via=me@there"])
        check("--var goat_* wins over the probe", (v2["probe_context"], v2["probe_via"]), ("mine", "me@there"))
        os.unlink(vf.name)
        # unresolved-var guard: a row with an unresolved {{ }} must be UNKNOWN
        r = resolve({"url": "https://{{ missing_var }}/x"}, {"site_id": "zagros-prod"})
        check("unresolved->sentinel", "<UNRESOLVED:missing_var>" in r["url"], True)
        # batch expansion: for_each over a good list PASSes, a bad item FAILs the batch
        vals = {"svcs": ["localhost"]}
        check("batch green", run_one(probes.cmd_assert, "cmd-assert",
              {"for_each": "svcs", "cmd_template": "echo {item}", "expect": "localhost"},
              {}, vals).status, probes.PASS)
        vals2 = {"svcs": ["localhost", "nope"]}
        check("batch RED (one bad)", run_one(probes.cmd_assert, "cmd-assert",
              {"for_each": "svcs", "cmd_template": "echo {item}", "expect": "localhost"},
              {}, vals2).status, probes.FAIL)
        check("batch UNKNOWN (empty list)", run_one(probes.dns_resolve, "dns-resolve",
              {"names_from": "missing"}, {}, {}).status, probes.UNKNOWN)
        # A control-node .py runs under this runner's interpreter, whatever
        # python3 comes first on PATH; a module missing on this machine is
        # UNKNOWN (X-PY-001), never a FAIL of the goatnet.
        script = Path(d) / "which-python.py"
        script.write_text("#!/usr/bin/env python3\nimport sys\nprint(sys.executable)\n")
        script.chmod(0o755)
        check("py script under the runner's python", probes.cmd_assert(
              {"host": "localhost", "cmd": str(script), "expect": sys.executable}).status, probes.PASS)
        nomod = Path(d) / "needs-module.py"
        nomod.write_text("#!/usr/bin/env python3\nimport goat_assay_no_such_module\n")
        nomod.chmod(0o755)
        check("py script missing a module -> UNKNOWN", probes.cmd_assert(
              {"host": "localhost", "cmd": str(nomod), "expect": "OK"}).status, probes.UNKNOWN)
        # a health envelope arrives as base64 from the REST plugin and as
        # `hex:` from `goat sub`; both decode to the same site.
        _env = bytes.fromhex("0a046d657368120a7a6167726f732d646576")  # EnvHealthV1{domain:"mesh", scope:"zagros-dev"}
        import base64 as _b64
        check("payload from goat sub (hex:)", probes._payload_fields("hex:" + _env.hex()).get("site"), "zagros-dev")
        check("payload from REST (base64)", probes._payload_fields(_b64.b64encode(_env).decode()).get("site"), "zagros-dev")
        # goat_cli: a stub `goat` on PATH plays each answer the real one gives.
        for name, got, want in goat_cli_self_test(Path(d)):
            check(name, got, want)
        # must_refuse inversion, driven through run() itself so the loop that
        # crashed in #1130 is the code under test: a refused probe is the PASS,
        # an answered probe is the FAIL, and UNKNOWN stays UNKNOWN.
        for name, got, want in must_refuse_self_test(Path(d)):
            check(name, got, want)
        # oidc-flow, bus-roundtrip and enroll-probe against stub services.
        for name, got, want in live_probe_self_test(Path(d)):
            check(name, got, want)

    # --plan lists the rows the run grades: a row needing a declared-absent service
    # is N/A in the plan as it is in the run (LP-015), and applicable without a model.
    rows = [{"id": "prom-up", "aspect": "obs/prometheus", "requires": ["prometheus"]},
            {"id": "mgmt-up", "aspect": "mgmt-stack/netbird", "requires": ["netbird-mgmt"]}]
    for name, absent, want in (("plan: an absent service's row is N/A", {"prometheus"}, "N/A here: prom-up"),
                               ("plan: with no model every row applies", set(), "2 applicable rows, 0 N/A")):
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            plan(argparse.Namespace(env="selftest"), rows, {}, absent)
        check(name, want in buf.getvalue(), True)

    print(f"\nself-test: {'ALL PROBES PROVEN RED-THEN-GREEN' if ok else 'FAILURES ABOVE'}")
    return 0 if ok else 1


def goat_cli_self_test(tmp):
    """The goat-cli probe against a stub `goat` whose answer is chosen by the
    verb: the green, the jq-false red, the probe-locus UNKNOWN (no profile, a
    goat too old for the verb), and a goatnet refusal graded FAIL."""
    stub = tmp / "bin" / "goat"
    stub.parent.mkdir(exist_ok=True)
    stub.write_text(
        "#!/bin/sh\n"
        "[ \"$1\" = --version ] && { echo \"goat ${goat_stub_version:-0.5.14}\"; exit 0; }\n"
        "case \"$3\" in\n"
        "  green) echo '{\"beat\": true, \"overall_color\": \"green\"}' ;;\n"
        "  red) echo '{\"beat\": false}' ;;\n"
        "  noprofile) echo 'error: no profile for context `x`' >&2; echo '  code: E_PROFILE_MISSING' >&2; exit 3 ;;\n"
        "  old) echo \"error: unexpected argument '--json' found\" >&2; exit 2 ;;\n"
        "  norouter) echo 'error: could not open a fabric session' >&2; echo '  code: E_ROUTER_UNREACHABLE' >&2; exit 4 ;;\n"
        "  noportal) echo 'error: portal unreachable' >&2; echo '  code: E_PORTAL_UNREACHABLE' >&2; exit 4 ;;\n"
        "  offmesh) echo 'error: not on the mesh' >&2; exit 5 ;;\n"
        "  notfound) echo 'error: the portal answered 404 for /api/x' >&2; echo '  code: E_PORTAL_NOT_FOUND' >&2; exit 7 ;;\n"
        "  busy) echo 'error: the portal answered 429' >&2; exit 8 ;;\n"
        "  refused) echo 'error: the portal refused the request (HTTP 422)' >&2; echo '  code: E_PORTAL_REFUSED' >&2; exit 4 ;;\n"
        "  legsok) echo '{\"beat\": true, \"overall_color\": \"green\"}'; exit 1 ;;\n"
        "  nojson) echo 'error: something broke' >&2; exit 1 ;;\n"
        "  empty) echo '[]' ;;\n"
        "esac\n")
    stub.chmod(0o755)
    path = os.environ.get("PATH", "")
    os.environ["PATH"] = f"{stub.parent}:{path}"
    try:
        jq = ".beat and .overall_color == \"green\""
        if not shutil.which("jq"):
            yield "goat-cli (skipped: no jq)", probes.PASS, probes.PASS
            return
        for name, verb, want in [("goat-cli green", "green", probes.PASS),
                                 ("goat-cli RED (no beat)", "red", probes.FAIL),
                                 ("goat-cli UNKNOWN (no profile)", "noprofile", probes.UNKNOWN),
                                 ("goat-cli UNKNOWN (goat lacks the flag)", "old", probes.UNKNOWN),
                                 ("goat-cli UNKNOWN (router not reached)", "norouter", probes.UNKNOWN),
                                 ("goat-cli UNKNOWN (portal not reached)", "noportal", probes.UNKNOWN),
                                 ("goat-cli UNKNOWN (off the mesh, no code)", "offmesh", probes.UNKNOWN),
                                 ("goat-cli RED (portal answered 404)", "notfound", probes.FAIL),
                                 ("goat-cli RED (portal answered 429, no code)", "busy", probes.FAIL),
                                 ("goat-cli RED (portal refused)", "refused", probes.FAIL)]:
            v = probes.goat_cli({"context": "x", "verb": verb, "expect_jq": jq})
            yield name, v.status, want
        # expect_rc graded-by-json: the JSON decides a non-zero exit; no JSON fails.
        v = probes.goat_cli({"context": "x", "verb": "legsok", "expect_jq": jq, "expect_rc": "graded-by-json"})
        yield "goat-cli green (exit 1, JSON graded)", v.status, probes.PASS
        v = probes.goat_cli({"context": "x", "verb": "legsok", "expect_jq": jq})
        yield "goat-cli RED (exit 1 without graded-by-json)", v.status, probes.FAIL
        v = probes.goat_cli({"context": "x", "verb": "nojson", "expect_jq": jq, "expect_rc": "graded-by-json"})
        yield "goat-cli RED (exit 1, no JSON, graded-by-json)", v.status, probes.FAIL
        # unknown_jq: an empty answer that cannot grade the row is UNKNOWN.
        v = probes.goat_cli({"context": "x", "verb": "empty", "expect_jq": "length >= 1",
                             "unknown_jq": "length == 0", "unknown_reason": "idle or denied"})
        yield "goat-cli UNKNOWN (empty answer, unknown_jq)", v.status, probes.UNKNOWN
        v = probes.goat_cli({"context": "x", "verb": "empty", "expect_jq": "length >= 1"})
        yield "goat-cli RED (empty answer, no unknown_jq)", v.status, probes.FAIL
        # A goat older than the row's floor is UNKNOWN before it runs anything.
        probes._VERSIONS.clear()
        os.environ["goat_stub_version"] = "0.4.41"
        try:
            v = probes.goat_cli({"context": "x", "verb": "green", "expect_jq": jq})
            yield "goat-cli UNKNOWN (goat 0.4.41 below the floor)", v.status, probes.UNKNOWN
            v = probes.goat_cli({"context": "x", "verb": "green", "expect_jq": jq, "min_goat": "0.4.0"})
            yield "goat-cli green (row floor 0.4.0 admits 0.4.41)", v.status, probes.PASS
        finally:
            os.environ.pop("goat_stub_version", None)
            probes._VERSIONS.clear()
    finally:
        os.environ["PATH"] = path


MUST_REFUSE_FIXTURE = {"tests": [
    {"id": "path-is-live", "probe": "cmd-assert", "params": {"cmd": "true"}},
    {"id": "refused", "probe": "cmd-assert", "params": {"cmd": "exit 1"},
     "must_refuse": True, "paired_positive": "path-is-live"},
    {"id": "allowed", "probe": "cmd-assert", "params": {"cmd": "true"},
     "must_refuse": True, "paired_positive": "path-is-live"},
    {"id": "cannot-tell", "probe": "dns-resolve", "params": {"names_from": "missing"},
     "must_refuse": True, "paired_positive": "path-is-live"},
]}
MUST_REFUSE_WANT = {"refused": probes.PASS, "allowed": probes.FAIL, "cannot-tell": probes.UNKNOWN}


def must_refuse_self_test(tmp):
    """Run the fixture manifest through run() and yield (name, got, want) per
    must_refuse row. A crash in the runner is reported as the verdict, so the
    self-test prints a FAIL line instead of a traceback."""
    manifest, out = tmp / "must-refuse.yaml", tmp / "must-refuse.jsonl"
    manifest.write_text(yaml.safe_dump(MUST_REFUSE_FIXTURE))
    got = {}
    try:
        with contextlib.redirect_stdout(io.StringIO()):
            run(["--env", "self-test", "--manifest", str(manifest), "--json", str(out)])
        got = {r["id"]: r["status"] for r in map(json.loads, out.read_text().splitlines())}
    except Exception as e:  # noqa: BLE001
        got = dict.fromkeys(MUST_REFUSE_WANT, f"runner raised {type(e).__name__}: {e}")
    for vid, want in MUST_REFUSE_WANT.items():
        yield f"must_refuse {vid}", got.get(vid, "no verdict"), want


# --------------------------------------------------------------------------
# The three probes that talk to a live identity plane, fabric and mesh, each
# proven against stubs that answer the way the real services do: one local
# HTTP server plays Hydra (discovery, JWKS, the authorization endpoint, the
# admin API), the login UI, the router's REST plugin (server-sent events) and
# the NetBird management API; stub `goat` and `docker` binaries play the CLI
# and the container runtime. Every probe is shown green, red, and UNKNOWN or
# SKIPPED where it cannot speak for the goatnet.
# --------------------------------------------------------------------------
class _Stub:
    """Mutable behaviour for the stub server; each test case sets it."""
    clients = {"netbird-dashboard": {"client_id": "netbird-dashboard",
                                     "redirect_uris": ["https://dash.example/cb"]}}
    jwks_keys = [{"kid": "k1"}]
    login_for_any = False      # an issuer that starts a login for any client id
    login_ui_status = 200
    beat = None                # JSON beat served on ops/mesh/health/<site>
    puts = {}                  # key -> last PUT body (the REST round trip)
    peers = {}                 # NetBird peers by id
    peer_delete_status = 200
    setup_key = "VALID-KEY"


def _stub_server():
    import http.server
    import threading
    import time
    import urllib.parse

    class H(http.server.BaseHTTPRequestHandler):
        def log_message(self, *a):
            pass

        def send(self, code, body=b"", ctype="application/json", headers=()):
            self.send_response(code)
            self.send_header("Content-Type", ctype)
            for k, v in headers:
                self.send_header(k, v)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def do_GET(self):  # noqa: N802
            u = urllib.parse.urlparse(self.path)
            base = f"http://127.0.0.1:{self.server.server_port}"
            if u.path == "/.well-known/openid-configuration":
                return self.send(200, json.dumps({
                    "issuer": base + "/", "jwks_uri": base + "/.well-known/jwks.json",
                    "authorization_endpoint": base + "/oauth2/auth",
                    "token_endpoint": base + "/oauth2/token"}).encode())
            if u.path == "/.well-known/jwks.json":
                return self.send(200, json.dumps({"keys": _Stub.jwks_keys}).encode())
            if u.path == "/oauth2/auth":
                cid = urllib.parse.parse_qs(u.query).get("client_id", [""])[0]
                if cid in _Stub.clients or _Stub.login_for_any:
                    return self.send(302, headers=[("Location", base + "/login?login_challenge=abc")])
                return self.send(302, headers=[("Location", base + "/error?error=invalid_client")])
            if u.path == "/login":
                return self.send(_Stub.login_ui_status, b"login", "text/html")
            if u.path == "/admin/clients":
                return self.send(200, json.dumps(list(_Stub.clients.values())).encode())
            if u.path == "/@/local/router":
                return self.send(200, b"[]")
            if u.path.startswith("/ops/") or u.path.startswith("/volatile/"):
                key = u.path.lstrip("/")
                deadline = time.time() + 6
                while time.time() < deadline:
                    if key.startswith("ops/") and _Stub.beat is not None:
                        val = _Stub.beat
                        break
                    if key in _Stub.puts:
                        val = _Stub.puts[key]
                        break
                    time.sleep(0.2)
                else:
                    return self.send(200, b"", "text/event-stream")
                ev = f"event:PUT\ndata:{json.dumps({'key': key, 'value': val})}\n\n".encode()
                return self.send(200, ev, "text/event-stream")
            if u.path == "/api/peers":
                return self.send(200, json.dumps(list(_Stub.peers.values())).encode())
            return self.send(404, b"{}")

        def do_PUT(self):  # noqa: N802
            n = int(self.headers.get("Content-Length") or 0)
            body = self.rfile.read(n).decode()
            _Stub.puts[urllib.parse.urlparse(self.path).path.lstrip("/")] = body
            return self.send(200, b"")

        def do_POST(self):  # noqa: N802
            n = int(self.headers.get("Content-Length") or 0)
            req = json.loads(self.rfile.read(n) or b"{}")
            if self.path == "/api/setup-keys":
                _Stub.peers = {}
                return self.send(200, json.dumps({"id": "k1", "key": _Stub.setup_key,
                                                  "name": req.get("name")}).encode())
            return self.send(404, b"{}")

        def do_DELETE(self):  # noqa: N802
            if self.path.startswith("/api/peers/"):
                pid = self.path.rsplit("/", 1)[1]
                if _Stub.peer_delete_status == 200:
                    _Stub.peers.pop(pid, None)
                return self.send(_Stub.peer_delete_status, b"{}")
            if self.path.startswith("/api/setup-keys/"):
                return self.send(200, b"{}")
            return self.send(404, b"{}")

    srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), H)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    return srv


_DOCKER_STUB = r"""#!/usr/bin/env python3
# A stub `docker`: run records a container and the setup key it was given;
# exec answers `netbird status --json` (enrolled only with the stub's valid
# key, unless goat_stub_any_key is set) and the reach check (goat_stub_reach); rm drops
# the container and, for an enrolled one, registers it as a NetBird peer
# through the stub API so cleanup has something to delete.
import json, os, sys, urllib.request
d = os.environ["goat_stub_dir"]; a = sys.argv[1:]
def path(n): return os.path.join(d, n + ".json")
if a[0] == "run":
    name = a[a.index("--name") + 1]; host = a[a.index("--hostname") + 1]
    key = next(x.split("=", 1)[1] for x in a if x.startswith("NB_SETUP_KEY="))
    ok = key == os.environ["goat_stub_key"] or os.environ.get("goat_stub_any_key")
    json.dump({"host": host, "ok": bool(ok)}, open(path(name), "w"))
    if ok and not name.endswith("-neg"):
        req = urllib.request.Request(os.environ["goat_stub_api"] + "/stub-register", method="POST",
                                     data=json.dumps({"hostname": host}).encode())
        try: urllib.request.urlopen(req)
        except Exception: pass
    print("cid"); sys.exit(0)
if a[0] == "exec":
    c = json.load(open(path(a[1])))
    if a[2:5] == ["netbird", "status", "--json"]:
        print(json.dumps({"management": {"connected": c["ok"]},
                          "netbirdIp": "100.127.0.60/16" if c["ok"] else ""})); sys.exit(0)
    sys.exit(0 if os.environ.get("goat_stub_reach") == "1" else 1)
if a[0] == "rm":
    try: os.remove(path(a[-1]))
    except OSError: pass
    sys.exit(0)
sys.exit(2)
"""

_GOAT_STUB = r"""#!/bin/sh
# A stub `goat`: `sub KEY ...` prints one sample chosen by goat_stub_goat, or the
# nonce the stub `pub` wrote; `pub KEY --payload P` records P.
verb="$3"; key="$4"
case "$verb" in
  pub) printf '%s' "$6" > "$goat_stub_dir/goat-pub"; exit 0 ;;
  sub)
    case "$goat_stub_goat" in
      beat) echo "$key {\"site\": \"$goat_stub_site\", \"scope\": \"$goat_stub_site\"}" ;;
      silent) echo "error: no sample before the deadline" >&2; exit 1 ;;
      noprofile) echo 'error: no profile' >&2; echo '  code: E_PROFILE_MISSING' >&2; exit 3 ;;
      roundtrip) sleep 3; echo "$key $(cat "$goat_stub_dir/goat-pub" 2>/dev/null)" ;;
    esac ;;
esac
"""


def live_probe_self_test(tmp):
    srv = _stub_server()
    base = f"http://127.0.0.1:{srv.server_port}"
    # the stub API's peer registration (called by the stub docker on enrol)
    handler = srv.RequestHandlerClass
    orig_post = handler.do_POST

    def do_post(self):  # noqa: N802
        if self.path == "/stub-register":
            n = int(self.headers.get("Content-Length") or 0)
            host = json.loads(self.rfile.read(n))["hostname"]
            _Stub.peers["p1"] = {"id": "p1", "hostname": host, "name": host}
            return self.send(200, b"{}")
        return orig_post(self)
    handler.do_POST = do_post

    bindir = tmp / "live-bin"
    bindir.mkdir(exist_ok=True)
    (bindir / "docker").write_text(_DOCKER_STUB)
    (bindir / "goat").write_text(_GOAT_STUB)
    for f in ("docker", "goat"):
        (bindir / f).chmod(0o755)
    saved = {k: os.environ.get(k) for k in ("PATH", "goat_stub_dir", "goat_stub_key", "goat_stub_api", "goat_stub_goat",
                                            "goat_stub_site", "goat_stub_reach", "goat_stub_any_key", "goat_stub_nb_token")}
    os.environ.update(PATH=f"{bindir}:{os.environ.get('PATH', '')}", goat_stub_dir=str(tmp),
                      goat_stub_key=_Stub.setup_key, goat_stub_api=base, goat_stub_nb_token="t")
    try:
        # ---- oidc-flow ----
        cfg = tmp / "management.json"
        oidc = {"issuer": base + "/", "host": "localhost", "admin_url": base,
                "client": "netbird-dashboard", "assert_nonempty": ["AuthIssuer", "AuthAudience"],
                "config_file": str(cfg), "timeout_s": 5}
        cfg.write_text(json.dumps({"HttpConfig": {"AuthIssuer": base + "/", "AuthAudience": "netbird-dashboard"}}))
        yield "oidc-flow green", probes.oidc_flow(oidc).status, probes.PASS
        cfg.write_text(json.dumps({"HttpConfig": {"AuthIssuer": base + "/", "AuthAudience": ""}}))
        yield "oidc-flow RED (empty AuthAudience)", probes.oidc_flow(oidc).status, probes.FAIL
        cfg.write_text(json.dumps({"HttpConfig": {"AuthIssuer": base + "/", "AuthAudience": "netbird-dashboard"}}))
        yield "oidc-flow RED (client not in Hydra)", probes.oidc_flow(dict(oidc, client="microscope")).status, probes.FAIL
        _Stub.jwks_keys = []
        yield "oidc-flow RED (no signing keys)", probes.oidc_flow(oidc).status, probes.FAIL
        _Stub.jwks_keys = [{"kid": "k1"}]
        _Stub.login_ui_status = 502
        yield "oidc-flow RED (login UI down)", probes.oidc_flow(oidc).status, probes.FAIL
        _Stub.login_ui_status = 200
        _Stub.login_for_any = True
        yield "oidc-flow RED (cannot refuse a bogus client)", probes.oidc_flow(oidc).status, probes.FAIL
        _Stub.login_for_any = False
        yield "oidc-flow UNKNOWN (issuer unreachable)", probes.oidc_flow(
            dict(oidc, issuer="http://127.0.0.1:9/")).status, probes.UNKNOWN

        # ---- bus-roundtrip: through goat, then through the router's REST ----
        sub = {"subscribe": "ops/mesh/health/e2e", "expect_field": {"site": "e2e"},
               "timeout_s": 4, "context": "x", "via": "localhost"}
        os.environ.update(goat_stub_goat="beat", goat_stub_site="e2e")
        yield "bus goat green", probes.bus_roundtrip(sub).status, probes.PASS
        os.environ["goat_stub_site"] = "zagros"
        yield "bus goat RED (another site's beat)", probes.bus_roundtrip(sub).status, probes.FAIL
        os.environ["goat_stub_goat"] = "silent"
        yield "bus goat RED (nothing heard)", probes.bus_roundtrip(sub).status, probes.FAIL
        os.environ["goat_stub_goat"] = "noprofile"
        yield "bus UNKNOWN (no transport)", probes.bus_roundtrip(sub).status, probes.UNKNOWN
        rest = dict(sub, rest_host="localhost", rest_port=srv.server_port)
        import base64
        beat = bytes([0x0a, 4]) + b"mesh" + bytes([0x12, 3]) + b"e2e" + bytes([0x22, 5]) + b"green"
        _Stub.beat = base64.b64encode(beat).decode()
        yield "bus REST green (EnvHealthV1)", probes.bus_roundtrip(rest).status, probes.PASS
        _Stub.beat = base64.b64encode(beat.replace(b"\x12\x03e2e", b"\x12\x04zagros")).decode()
        yield "bus REST RED (scope is another site)", probes.bus_roundtrip(rest).status, probes.FAIL
        _Stub.beat = None
        yield "bus REST RED (nothing heard)", probes.bus_roundtrip(rest).status, probes.FAIL
        rt = {"publish": True, "key": "volatile/goat-assay/e2e", "timeout_s": 6, "context": "x",
              "via": "localhost", "settle_s": 0.5}
        os.environ["goat_stub_goat"] = "roundtrip"
        yield "bus goat round trip green", probes.bus_roundtrip(rt).status, probes.PASS
        yield "bus REST round trip green", probes.bus_roundtrip(
            dict(rt, context="", rest_host="localhost", rest_port=srv.server_port)).status, probes.PASS

        # ---- enroll-probe ----
        en = {"part": "enroll", "opt_in": "1", "mgmt_url": base, "peer_host": "localhost",
              "image": "netbirdio/netbird:test", "token_env": "goat_stub_nb_token",
              "reach_ip": "100.127.0.1", "timeout_s": 4, "negative_wait_s": 0}

        def enroll(part="enroll", **kw):
            probes._ENROLL_SESSIONS.clear()
            for k, v in kw.items():
                if k.startswith("goat_stub_"):
                    os.environ[k] = v
            v = probes.enroll_probe(dict(en, part="enroll", **{k: v for k, v in kw.items()
                                                              if not k.startswith("goat_stub_")}))
            return v if part == "enroll" else probes.enroll_probe(dict(en, part=part))
        yield "enroll SKIPPED unless opted in", probes.enroll_probe(dict(en, opt_in="false")).status, probes.SKIPPED
        yield "enroll UNKNOWN without the admin token", probes.enroll_probe(
            dict(en, token_env="goat_stub_no_such_token")).status, probes.UNKNOWN
        os.environ.pop("goat_stub_any_key", None)
        yield "enroll green", enroll(goat_stub_reach="1").status, probes.PASS
        yield "enroll mesh-reach green", probes.enroll_probe(dict(en, part="mesh-reach")).status, probes.PASS
        yield "enroll cleaned up (no peer left)", not _Stub.peers, True
        yield "enroll mesh-reach RED", enroll("mesh-reach", goat_stub_reach="0").status, probes.FAIL
        yield "enroll RED (a made-up key enrols)", enroll(goat_stub_any_key="1").status, probes.FAIL
        os.environ.pop("goat_stub_any_key", None)
        _Stub.peer_delete_status = 500
        v = enroll(goat_stub_reach="1")
        yield "enroll RED (cleanup left the peer)", (v.status, "cleanup incomplete" in v.evidence), (probes.FAIL, True)
        _Stub.peer_delete_status = 200
    finally:
        srv.shutdown()
        for k, v in saved.items():
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v


if __name__ == "__main__":
    sys.exit(run())
