#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
#
# report.py — the parity report, STAGE 1 (intent parity, repo-side only).
#
# Renders "what is each governed goatnet SUPPOSED to run", as a component x node
# matrix from ops/pins/fleet-pins.yaml plus a curated set of capability flags
# from ops/pins/parity-capabilities.yaml — the question that otherwise takes
# archaeology across ~16 var files.
#
# Canon: docs/design/environment-parity.md §5 (stage 1). The design doc names
# the operator-facing path `ops/scripts/parity-report.sh`; that exists and is a
# thin wrapper on this file, so the pin machinery stays in one directory.
#
# WHAT THIS IS NOT. This report reads the COMMITTED TREE. It does not contact
# any host, so it cannot tell you what is running anywhere. Stage 2 (design doc
# §5, queued) joins the on-host /var/lib/goat/<component>/deploy-history.jsonl
# tips and adds the intra-goatnet node-skew view; until then "the repo says X"
# is the only claim this file is entitled to make, and it says so on every
# render.
#
# HONESTY RULES (design doc §2 principle 4, binding):
#   - A value the report cannot resolve renders `unreadable`, never OK. A row
#     with any unreadable cell grades `unreadable`, never `match`.
#   - A key with no answer anywhere renders `unset` — distinct from unreadable
#     (nothing to read vs. read it and could not evaluate it).
#   - Nothing is silently omitted: pins outside the fleet table render as
#     UNGOVERNED rows, retired keys render as absence assertions, and the
#     goatnets this report does not govern are named rather than dropped.
#   - No percentages. A ratio over a set this small invents precision.
#
# DOWNGRADED BY CF-3 (consistency-foundation.md §5.4). This file used to WRITE a
# committed render, `ops/pins/parity-report.md`, gated by the fleet-pins CI job.
# That file is deleted and that gate now sits on `ops/consistency/intent/<g>/
# intent.json`, which is what it was always really gating: a resolved,
# per-member, schema-validated answer to "what is this goatnet supposed to run",
# readable by consumers that hold no checkout.
#
# What survives is the RENDER, on demand, to stdout — because a human-legible
# matrix of the same facts is still the fastest way to answer the question in a
# terminal, and it costs nothing to keep once it owns no file and no gate. What
# does NOT survive is any committed output: a second committed rendering of the
# artifact would be exactly the "one fact, two homes" drift this arc exists to
# remove, and the one that goes stale is always the prose one.
#
# Modes:
#   (default)   render to stdout, write nothing
#   --stdout    the same thing, named, for scripts that want to be explicit
#
# Exit codes: 0 ok, 2 usage or schema error.

import argparse
import glob
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
REPO_ROOT = os.path.normpath(os.path.join(HERE, "..", ".."))
GROUP_VARS = os.path.join(REPO_ROOT, "ops/ansible/group_vars")

# Resolution AND grading live in ops/consistency/python/goat_consistency/ and are
# imported. This file is a RENDERER: it lays out markdown and nothing else. It
# does not decide what a value is, and since CF-2 it does not decide what a
# difference MEANS either — a projection may choose what it can see and how it
# renders, never how it grades (ADR 1110 D2).
#
# Before CF-2 the resolution semantics were reachable only through
# importlib.util.spec_from_file_location on a hyphenated script. That reach is
# gone; a package that can be imported is the whole point.
sys.path.insert(0, os.path.join(REPO_ROOT, "ops", "consistency", "python"))

try:
    from goat_consistency import GrammarError, SchemaError, grading, resolve  # noqa: E402
    from goat_consistency import intent as intent_mod  # noqa: E402
except ImportError as exc:  # pragma: no cover - dependency, not logic
    sys.stderr.write("ERROR: %s (PyYAML required: pip install pyyaml)\n" % exc)
    sys.exit(2)

all_scope_vars = resolve.all_scope_vars
load_caps = resolve.load_caps
read_yaml = resolve.read_yaml
resolve_var = resolve.resolve_var
resolve_var_full = resolve.resolve_var_full
role_defaults = resolve.role_defaults
scope_vars = resolve.scope_vars

# group_vars dirs that are not goatnet mgmt-node member groups. Named so the
# "not shown here" list below stays about environments rather than plumbing.
NON_ENV_GROUP_VARS = {"_examples", "all.yml"}


def rel(path):
    return os.path.relpath(path, REPO_ROOT)


def cell(text):
    """Markdown table cells cannot carry a raw pipe; jinja filters are full of them."""
    return str(text).replace("|", "\\|")


# ---------------------------------------------------------------------------
# rendering helpers
# ---------------------------------------------------------------------------

def short_ref(ref):
    """Matrix-width image ref. The full ref is printed under the matrix — this
    truncates the digest for LAYOUT only and marks the truncation."""
    s = ref.replace("ghcr.io/{{ ghcr_org }}/", "")
    if "@sha256:" in s:
        head, digest = s.split("@sha256:", 1)
        return "%s@sha256:%s…" % (head, digest[:12])
    return s


def grade(cells_by_member, goatnet, declared=False, artifact_type=None):
    """The row grade, from the library — via the same GradedRow every other
    surface will get, of which this renderer prints exactly one field.

    `why`, `remediation` and `grounding` are computed here too and deliberately
    not rendered: the stage-1 markdown predates them, and CF-2's acceptance is
    that this file's output does not move. They are pinned by the golden corpus
    instead, which is where a change to them belongs."""
    return grading.grade_row("row", "row", cells_by_member, goatnet,
                             vantage="repo", declared=declared,
                             artifact_type=artifact_type).grade


# ---------------------------------------------------------------------------
# rendering
# ---------------------------------------------------------------------------

def render(goatnets, components, caps, member_of):
    all_scope = all_scope_vars()
    defaults = role_defaults()
    scopes = {m: scope_vars(os.path.join(GROUP_VARS, m)) for m in member_of}

    unreadable_notes = []
    L = []
    A = L.append

    A("<!-- Rendered on demand by ops/pins/report.py. Not a committed file. -->")
    A("# Parity report — stage 1 (intent)")
    A("")
    A("**RENDERED ON DEMAND** by `ops/pins/report.py` (via `ops/scripts/parity-report.sh --stage1`).")
    A("Nothing is written; this render is exactly as current as the checkout it ran against.")
    A("The committed, `--check`-gated form of these facts is")
    A("`ops/consistency/intent/<goatnet>/intent.json` — the artifact that replaced this file's")
    A("predecessor, `ops/pins/parity-report.md`, when CF-3 landed")
    A("([consistency-foundation.md](../../docs/design/consistency-foundation.md) §5.4).")
    A("")
    A("## What this report is, and is not")
    A("")
    A("This is a **repo-side desired-state report**. Every value below is read out of the committed")
    A("tree — `ops/pins/fleet-pins.yaml`, `ops/ansible/group_vars/`, role defaults. **It does not")
    A("contact any host and therefore does not know what is running anywhere.** A green-looking row")
    A("means the repo is internally consistent, not that a deploy happened.")
    A("")
    A("Joining this against live truth is **stage 2** (design doc §5): read each node's")
    A("`/var/lib/goat/<component>/deploy-history.jsonl` tip, diff it against the intent below, and")
    A("render the intra-goatnet node-skew view. Queued, not built.")
    A("")
    A("Honesty grammar used throughout (design doc §2 principle 4):")
    A("")
    A("| Grade | Meaning |")
    A("|---|---|")
    A("| `match` | Every member of the goatnet resolves to the same value. |")
    A("| `SKEW` | Members resolve to different values and no deviation declares it. |")
    A("| `declared` | Members differ, and `ops/pins/parity-capabilities.yaml` (or `fleet-pins.yaml`) says why. |")
    A("| `unreadable` | At least one value was read but could not be evaluated (templated, or defaulted by several roles). **Never `match`.** |")
    A("| `indeterminate` | At least one member is `unset`, so equality cannot be judged. **Never `match`.** |")
    A("| `unset` (cell) | No group_vars entry and no role default. The consuming role or template supplies its own default, which this report does not evaluate — so this is *not* the same as `false`. |")
    A("")
    A("`unreadable` and `indeterminate` are kept apart deliberately: one means the tree was read and")
    A("could not be evaluated, the other means there was nothing to read. Collapsing them would let a")
    A("missing key hide behind a templating excuse.")
    A("")
    A("Scope is **forward-only** (operator decision, 2026-08-02): only goatnets listed in")
    A("`ops/pins/fleet-pins.yaml` appear. Environments this report does not govern are named in the")
    A("last section rather than dropped.")

    # ---- per goatnet ------------------------------------------------------
    for gn in sorted(goatnets):
        members = list(goatnets[gn].get("members") or [])
        A("")
        A("---")
        A("")
        A("## Goatnet `%s`" % gn)
        A("")
        desc = str(goatnets[gn].get("description") or "").strip()
        if desc:
            A(" ".join(desc.split()))
            A("")
        A("Members (mgmt-node host-groups): %s" % ", ".join("`%s`" % m for m in members))

        # -- node shape -----------------------------------------------------
        A("")
        A("### Node shape (ungraded — per-node by design)")
        A("")
        A("Rendered FIRST because it explains which asymmetries below are structural. These are not")
        A("compared: ADR 1084 makes a federated goatnet's nodes deliberately unequal in role.")
        A("")
        A("| Var | " + " | ".join("`%s`" % m for m in members) + " |")
        A("|---|" + "---|" * len(members))
        for row in caps.get("node_shape") or []:
            var = row["var"]
            cells = []
            for m in members:
                disp, _, _ = resolve_var(var, scopes[m], all_scope, defaults)
                cells.append(cell(disp))
            A("| `%s` | %s |" % (var, " | ".join(cells)))

        # -- pins -----------------------------------------------------------
        A("")
        A("### Component pins (governed by `ops/pins/fleet-pins.yaml`)")
        A("")
        A("Digests are truncated to 12 hex characters for table width; the full refs follow.")
        A("")
        A("| Component | Var | " + " | ".join("`%s`" % m for m in members) + " | Node parity |")
        A("|---|---|" + "---|" * len(members) + "---|")
        for cname, spec in components.items():
            if resolve.opted_out(spec, gn):
                continue
            texts, graded, declared = [], {}, False
            for m in members:
                value, dev = resolve.resolve(spec, m, gn)
                if value is None:
                    texts.append("**absent** (declared)")
                    graded[m] = {"state": "absent"}
                    declared = True
                else:
                    suffix = " (deviation)" if dev is not None else ""
                    if dev is not None:
                        declared = True
                    texts.append(cell(short_ref(value)) + suffix)
                    graded[m] = {"state": "resolved", "value": value}
            g = grade(graded, gn, declared=declared,
                      artifact_type=resolve.artifact_type(spec))
            A("| %s | `%s` | %s | %s |" % (cname, spec["var"], " | ".join(texts), g))

        A("")
        A("Full refs and the reason behind every deviation:")
        A("")
        for cname, spec in components.items():
            if resolve.opted_out(spec, gn):
                continue
            A("- **%s** (`%s`)" % (cname, spec["var"]))
            for m in members:
                value, dev = resolve.resolve(spec, m, gn)
                if value is None:
                    A("  - `%s`: **not deployed here** — %s"
                      % (m, " ".join(str(dev.get("reason")).split())))
                    continue
                A("  - `%s`: `%s`" % (m, value))
                if dev is not None:
                    A("    - DEVIATION: %s" % " ".join(str(dev.get("reason")).split()))
                    if dev.get("until"):
                        A("    - until: %s" % dev["until"])
                    if resolve.deviation_scope(spec, m, gn) == m:
                        # ADR 1256: what this member deviation overrides. validate()
                        # already refused a stale or missing one, so the two agree.
                        A("    - over: `%s` (goatnet `%s` resolves `%s` without it)"
                          % (dev.get("over"), gn, resolve.goatnet_resolution(spec, gn)))

        # -- ungoverned tail ------------------------------------------------
        A("")
        A("### UNGOVERNED pins (present in group_vars, not rows in the fleet table)")
        A("")
        A("These are real deploy pins that the one-fact-one-home mechanism does **not** yet cover.")
        A("They are listed rather than omitted: an image this report stays silent about is exactly")
        A("the drift it exists to prevent. Migration is worst-first and incremental (design doc §3);")
        A("the tail is ratcheted shrink-only by `ops/pins/unmigrated-pins.txt`.")
        A("")
        allow, _ = resolve.load_allowlist()
        outside = [(m, k, p) for m, k, p in resolve.scan_outside_pins(member_of)
                   if member_of.get(m) == gn]
        if not outside:
            A("None — every pin in this goatnet's group_vars is governed.")
        else:
            A("| Member | Var | Value | Declared in the tail? |")
            A("|---|---|---|---|")
            for m, k, path in sorted(outside):
                disp, _, _ = resolve_var(k, scopes[m], all_scope, defaults)
                state = "yes" if ("%s:%s" % (m, k)) in allow else "**NO — undeclared**"
                A("| `%s` | `%s` | %s | %s |" % (m, k, cell(disp), state))

        # -- capability flags ------------------------------------------------
        A("")
        A("### Capability flags")
        A("")
        A("The curated set from `ops/pins/parity-capabilities.yaml`, which carries the `why:` for")
        A("every row. `SKEW` here is a finding, not a verdict: a difference may well be deliberate,")
        A("but until it is declared with a reason, this report cannot tell deliberate from missed.")
        A("")
        A("| Var | " + " | ".join("`%s`" % m for m in members) + " | Node parity |")
        A("|---|" + "---|" * len(members) + "---|")
        cap_devs = caps.get("deviations") or {}
        skews, undecided, declared_rows = [], [], []
        for row in caps.get("capabilities") or []:
            var = row["var"]
            texts, graded = [], {}
            for m in members:
                r = resolve_var_full(var, scopes[m], all_scope, defaults)
                texts.append(cell(r.display))
                # The cell handed to the library is the SAME shape IntentV1
                # carries, so this renderer and a consumer holding only the
                # artifact run one comparison rather than two that agree until
                # they don't.
                graded[m] = intent_mod.var_cell(r)
                if r.state == "unreadable":
                    unreadable_notes.append("`%s` on `%s` — %s" % (var, m, r.display))
            scoped = cap_devs.get(var) or {}
            declares_this = bool(set(scoped) & (set(members) | {gn}))
            g = grade(graded, gn, declared=declares_this)
            if g == "declared":
                declared_rows.append((var, scoped))
            elif g == "SKEW":
                skews.append(var)
            elif g == "indeterminate":
                undecided.append(var)
            A("| `%s` | %s | %s |" % (var, " | ".join(texts), g))

        if declared_rows:
            A("")
            A("Declared capability deviations:")
            A("")
            for var, scoped in declared_rows:
                for scope, dev in sorted(scoped.items()):
                    A("- `%s` on `%s`: %s" % (var, scope, " ".join(str(dev.get("reason")).split())))
                    if dev.get("until"):
                        A("  - until: %s" % dev["until"])
        if skews or undecided:
            A("")
            if skews:
                A("**Undeclared skew in this goatnet:** %s — members resolve to different values and"
                  % ", ".join("`%s`" % v for v in skews))
                A("nothing says why.")
                A("")
            if undecided:
                A("**Undecidable in this goatnet:** %s — at least one member has no group_vars"
                  % ", ".join("`%s`" % v for v in undecided))
                A("entry and no role default, so the effective value comes from a default this report")
                A("does not evaluate. Setting the key explicitly on every member is what turns this")
                A("row into an answer.")
            A("")
            A("Either the difference is deliberate — declare it in `ops/pins/parity-capabilities.yaml`")
            A("under `deviations:` with a reason — or it is the one-node-left-behind shape this report")
            A("was built to surface. This report does not decide which; it refuses to render the")
            A("difference as fine.")

        # -- retired keys ----------------------------------------------------
        A("")
        A("### Retired keys (must stay absent)")
        A("")
        A("| Var | State | Retired because |")
        A("|---|---|---|")
        for row in caps.get("retired") or []:
            var = row["var"]
            present = [m for m in members if var in scopes[m]]
            if var in all_scope:
                present.append("group_vars/all.yml")
            state = "absent (ok)" if not present else "**PRESENT in %s**" % ", ".join(present)
            A("| `%s` | %s | %s |" % (var, state, cell(" ".join(str(row["why"]).split()))))

    # ---- capabilities that differ BETWEEN goatnets -------------------------
    # Everything above compares members INSIDE one goatnet, which is the F-272
    # skew shape. A capability that differs between goatnets had no row shape
    # at all, so a both-ways divergence was not merely undeclared — it could
    # not be written down. Ungraded on purpose: a cross-goatnet difference is
    # usually deliberate, and the value is that it is recorded once next to its
    # reason instead of being rediscovered from a blank panel.
    cross = caps.get("cross_goatnet") or []
    if cross:
        A("")
        A("---")
        A("")
        A("## Capabilities that differ between goatnets")
        A("")
        A("Not skew, and not graded. These are DECLARED differences, recorded because the")
        A("consequence lands on someone reading a dashboard rather than someone reading")
        A("group_vars: a panel that says *not measured* looks the same whether the probe is")
        A("absent by decision or absent by accident.")
        A("")
        A("| Capability | Flag | Present | Absent | Why | What a reader sees |")
        A("|---|---|---|---|---|---|")
        for row in cross:
            var = row.get("var")
            # A null flag is a real answer, not a gap: the capability is
            # stack-shaped (it exists because of which compose project runs)
            # and there is no boolean anywhere to point at. Saying "stack" is
            # more honest than leaving the cell blank, which reads as unknown.
            flag = "`%s`" % var if var else "_stack-shaped, no flag_"
            fmt = lambda k: ", ".join("`%s`" % g for g in (row.get(k) or [])) or "—"
            A("| %s | %s | %s | %s | %s | %s |" % (
                cell(str(row.get("capability", "?"))),
                flag, fmt("present"), fmt("absent"),
                cell(" ".join(str(row.get("why", "")).split())),
                cell(" ".join(str(row.get("consequence", "")).split()))))

    # ---- what could not be read ------------------------------------------
    A("")
    A("---")
    A("")
    A("## What this report could not read")
    A("")
    A("Stated explicitly rather than defaulted green (design doc §2 principle 4).")
    A("")
    A("1. **Live host state — everything.** No host is contacted. Whether any node is actually")
    A("   running the pin above is unknown here and is stage 2's question.")
    A("2. **The registry org in every pin.** Refs carry a literal `{{ ghcr_org }}`, resolved at")
    A("   deploy time from the deploying checkout's git origin (`group_vars/all.yml`). That is")
    A("   deliberate — the same committed string must resolve to `goat-maintainer` or `usareur-af`")
    A("   depending on which checkout deploys — so it is left un-evaluated rather than guessed.")
    A("   The discriminating part of each ref (tag and digest) is literal and is shown in full.")
    A("3. **Multi-arch index-ness of any digest.** F-245 requires index (list) digests, never")
    A("   per-arch leaves. That cannot be determined offline; it is asserted at bump time against")
    A("   the registry and recorded in `fleet-pins.yaml`'s per-component comments.")
    A("4. **Anything set outside group_vars.** Inventory host_vars, `-e` extra-vars and runtime")
    A("   mutation are invisible here. That gap is the F-272/F-258 shape and is why the")
    A("   persistent-state rule (design doc §7) exists.")

    if unreadable_notes:
        A("5. **Values read but not evaluable:**")
        A("")
        for note in sorted(set(unreadable_notes)):
            A("   - %s" % note)
    else:
        A("5. **Values read but not evaluable:** none in this render — every tracked flag resolved")
        A("   to a literal or to a declared absence.")

    # currency model — design doc §5 adjacent one-liner
    declares_currency = sorted({
        os.path.basename(os.path.dirname(p))
        for p in glob.glob(os.path.join(GROUP_VARS, "*", "*.yml"))
        for k in read_yaml(p)
        if k == "currency_model" or k.endswith("_currency_model")
    })
    A("6. **Currency model per hosted surface** (`tracks-main` vs `pinned-release`, design doc §5's")
    if declares_currency:
        A("   adjacent one-liner): declared by %s." % ", ".join("`%s`" % d for d in declares_currency))
    else:
        A("   adjacent one-liner): **no group_vars key declares it anywhere in the tree**, so this")
        A("   report cannot print it. The mechanism is designed-not-built; the standing confusion")
        A("   between the two live-surface models is unresolved, not resolved-and-green.")

    ungoverned_envs = sorted(
        d for d in os.listdir(GROUP_VARS)
        if os.path.isdir(os.path.join(GROUP_VARS, d))
        and d not in NON_ENV_GROUP_VARS and d not in member_of
    )
    A("7. **Environments outside this report's scope**, named so their absence is a decision and")
    A("   not an oversight: %s." % ", ".join("`%s`" % d for d in ungoverned_envs))
    A("   Forward-only is binding — legacy environments are not retrofitted and can never fail the")
    A("   pin gate. One opts in by adding a `goatnets:` row to `ops/pins/fleet-pins.yaml`.")
    A("")
    A("Canon: [`docs/design/environment-parity.md`](../../docs/design/environment-parity.md) §5.")

    return "\n".join(L).rstrip("\n") + "\n"


def main():
    ap = argparse.ArgumentParser(
        description="Render the stage-1 (intent) parity matrix to stdout. Reads the "
                    "committed tree and contacts NO host; the live half is "
                    "`ops/scripts/parity-report.sh` (consistency-foundation.md §5.4).")
    ap.add_argument("--stdout", action="store_true",
                    help="render to stdout (the default; accepted for explicitness)")
    args = ap.parse_args()
    try:
        return run(args)
    except (SchemaError, GrammarError) as exc:
        sys.stderr.write("ERROR: %s\n" % exc)
        return 2


def run(args):
    goatnets, components = resolve.load_table()
    member_of = resolve.validate(goatnets, components)
    caps = load_caps()

    # CF-5 verify-at-read (ADR 1109 P14). This renderer re-resolves the tree
    # through the library rather than reading the artifact's body (§3.6, the
    # as-built correction) — but it renders the SAME intent the committed
    # artifact carries, so a committed artifact that fails verification means
    # the intent this page is about cannot currently be trusted, and the honest
    # output is the refusal, not the matrix.
    from goat_consistency import signing  # noqa: E402 (after sys.path insert)
    from goat_consistency.paths import intent_path  # noqa: E402
    for goatnet in sorted(goatnets):
        v = signing.verify_file(intent_path(goatnet))
        if v.state != "verified":
            sys.stderr.write("REFUSING to render: %s\n" % v.detail)
            return 1

    del args  # one mode; --stdout is accepted for explicitness and changes nothing
    for warning in resolve.redundant_member_deviations(goatnets, components, member_of):
        sys.stderr.write(warning + "\n")
    sys.stdout.write(render(goatnets, components, caps, member_of))
    return 0


if __name__ == "__main__":
    sys.exit(main())
