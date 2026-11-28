#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
#
# generate-pins.py — the CLI over ONE resolution pass with TWO renderings:
#
#   1. `ops/ansible/group_vars/<member>/pins.yml` for every member host-group of
#      every goatnet listed in ops/pins/fleet-pins.yaml (plus the lite sidecar
#      ops/pins/lite-pins.yaml, ADR 1213), so "what image does this goatnet
#      run" has ONE source of truth.
#   2. `ops/consistency/intent/<goatnet>/intent.json` — IntentV1, the typed
#      intent artifact every downstream consumer reads INSTEAD of re-resolving
#      the tree (docs/design/consistency-foundation.md §3, ADR 1110 D1).
#
# Two renderings from one resolver is the pattern ops/dns/generate-mesh-hosts.py
# already established when it emitted both the zone file and the name snapshot.
#
# THIS FILE IS A CLI, NOT A LIBRARY. Since CF-2 the resolution semantics live in
# ops/consistency/python/goat_consistency/ and are imported; what remains here
# is argument parsing, the pins.yml text rendering, the shrink-only ratchet on
# the unmigrated tail, and the exit codes. Anything that decides what a value IS
# belongs in the package, where a second implementation can be held to the same
# golden corpus.
#
# Canon: docs/design/environment-parity.md §3 + docs/design/consistency-
# foundation.md §3/§4. Modelled on ops/dns/generate-mesh-hosts.py — the F-197
# arc, where a hand-synced pair of files was first linted and then replaced by
# generation, and nothing in that class has recurred since.
#
# SCOPE IS FORWARD-ONLY. Only goatnets listed in fleet-pins.yaml are governed.
# Legacy envs are not listed, so nothing here can fail on their account.
#
# Modes:
#   (default)   rewrite every listed member's pins.yml from the table, and
#               every listed goatnet's intent.json
#   --check     validate the table, regenerate, and fail (exit 1) on any
#               difference; fail on a committed intent.json whose detached
#               offline-CA signature is missing or does not verify (CF-5,
#               design doc §3.4 — sign with ops/enrollment/cmd/intent-sign);
#               ALSO fail on a governed pin var defined outside
#               pins.yml anywhere in a listed goatnet's group_vars (the
#               shadowing hazard: within one group_vars dir the
#               later-loading file wins, and `vars.yml` sorts after
#               `pins.yml`, so a leftover hand pin silently shadows the
#               generated one). The unmigrated tail lives in
#               ops/pins/unmigrated-pins.txt and may only SHRINK.
#
# Exit codes: 0 ok, 1 drift/violation, 2 usage or schema error.

import argparse
import difflib
import os
import sys

REPO_ROOT = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))
sys.path.insert(0, os.path.join(REPO_ROOT, "ops", "consistency", "python"))

try:
    from goat_consistency import GrammarError, SchemaError, intent as intent_mod, resolve  # noqa: E402
    from goat_consistency import signing  # noqa: E402
    from goat_consistency.paths import GROUP_VARS, intent_path, rel  # noqa: E402
except ImportError as exc:  # pragma: no cover - dependency, not logic
    sys.stderr.write("ERROR: %s (PyYAML + jsonschema required: pip install pyyaml jsonschema)\n" % exc)
    sys.exit(2)

# The allowlist may only shrink (the check-rollback-coverage.sh ratchet). Its
# baseline is the `# baseline: <n>` line in ops/pins/unmigrated-pins.txt — one
# home, read by this script and by `goatbuilder pins` alike. Adding a row means
# raising that number, which is a visible, reviewable regression.

GENERATED_BASENAME = resolve.GENERATED_BASENAME

HEADER = """\
---
# {member} image pins. GENERATED FILE — do not hand-edit; regenerate via
# ops/pins/generate-pins.py.
#
# Rendered from ops/pins/fleet-pins.yaml, which is the ONE home for every
# governed `*_image` / release-tag pin of the `{goatnet}` goatnet. Rationale,
# rollback targets and deviation reasons live there, next to the value.
# Canon: docs/design/environment-parity.md §3.
#
# To bump an image: edit fleet-pins.yaml, rerun the generator, commit both. The
# `fleet-pins` CI job refuses drift between the table and this file, and refuses
# a governed pin defined anywhere else in this goatnet's group_vars.
#
# WHY the copies are generated rather than hand-kept:
{members_block}
#
# PRECEDENCE HAZARD (the reason a hand pin must not come back): within one
# group_vars directory ansible loads files alphabetically and the later file
# WINS on a key conflict. `vars.yml` sorts after `pins.yml`, so a pin re-added
# to vars.yml would silently shadow this file. That is what the CI job's
# outside-pins.yml scan exists to catch.

"""


def wrap_comment(text, width=76, indent="# "):
    out, line = [], indent
    for word in str(text).split():
        if len(line) + len(word) + 1 > width and line.strip() != indent.strip():
            out.append(line.rstrip())
            line = indent + word + " "
        else:
            line += word + " "
    if line.strip() != indent.strip():
        out.append(line.rstrip())
    return out


def render(member, goatnet, goatnets, components):
    peers = [m for m in goatnets[goatnet]["members"] if m != member]
    if peers:
        members_sentence = (
            "the `%s` goatnet's mgmt nodes are one fabric (design doc §7 "
            "node-parity rule). %s render from the same table row, so bumping "
            "one and missing the other is structurally impossible."
            % (goatnet, " and ".join([member] + peers))
        )
    else:
        members_sentence = (
            "the `%s` goatnet has one member today; the table row is still the "
            "single home so a second node inherits the pins by existing."
            % goatnet
        )
    body = HEADER.format(member=member, goatnet=goatnet,
                         members_block="\n".join(wrap_comment(members_sentence)))

    # Which goatnet governs this member, and which IntentV1 column it is —
    # rendered, not hand-copied. The `goatnets:` block above already declares
    # this mapping; a second copy in each vars.yml would be exactly the
    # one-fact-two-homes drift this table exists to prevent, and the drift would
    # be invisible because a wrong member_group reads as a node grading against
    # someone else's column rather than as an error.
    #
    # Read by the rollback-ledger shared role, which writes the applied-intent
    # stamp at /var/lib/goat/intent/applied.json (CF-3,
    # docs/design/consistency-foundation.md §5.2).
    lines = [
        "# --- consistency: which generation governs this node (CF-3) ---",
        'consistency_goatnet: "%s"' % goatnet,
        'consistency_member_group: "%s"' % member,
        "",
    ]

    # CF-4: the declared render watch-sets, keyed by LEDGER component (the
    # /var/lib/goat/<key>/ directory the manifest lives under). Rendered, not
    # hand-kept: fleet-pins.yaml is the one home of the declaration, and this
    # block is how it reaches the recording task
    # (rollback-ledger/tasks/render-manifest.yml). Paths may embed {{ vars }};
    # ansible resolves them lazily at record time, per host.
    watch = {}
    for cname, spec in components.items():
        paths = spec.get("render_watch") or []
        if not paths:
            continue
        value, _dev = resolve.resolve(spec, member, goatnet)
        if value is None:
            continue  # declared absent for this member — nothing to watch
        key = spec.get("ledger_component", cname) or cname
        watch[key] = paths
    if watch:
        lines.append("# --- consistency: declared config render watch-sets (CF-4) ---")
        lines.append("# Consumed by rollback-ledger/tasks/render-manifest.yml (default-off:")
        lines.append("# consistency_render_manifest_enabled). Declared in fleet-pins.yaml;")
        lines.append("# see docs/design/consistency-foundation.md §5.2a.")
        lines.append("consistency_render_watch:")
        for key in sorted(watch):
            lines.append('  "%s":' % key)
            for p in watch[key]:
                lines.append('    - "%s"' % p)
        lines.append("")
    for cname, spec in components.items():
        if resolve.opted_out(spec, goatnet):
            continue  # an opt-in component this goatnet did not opt in to: not named
        value, dev = resolve.resolve(spec, member, goatnet)
        if value is None:
            lines.append("# %s — NOT DEPLOYED here (declared `absent` in fleet-pins.yaml)." % cname)
            lines.extend(wrap_comment("Reason: " + str(dev.get("reason", "")).strip()))
            lines.append("")
            continue
        lines.append("# %s" % cname)
        if dev is not None:
            lines.extend(wrap_comment("DEVIATION from the fleet value. Reason: "
                                      + str(dev.get("reason", "")).strip()))
            if dev.get("until"):
                lines.append("# until: %s" % dev["until"])
        if spec.get("note"):
            lines.extend(wrap_comment(str(spec["note"]).strip()))
        lines.append('%s: "%s"' % (spec["var"], value))
        lines.append("")
    return body + "\n".join(lines).rstrip("\n") + "\n"


def run(args):
    tables = resolve.load_tables()
    goatnets, components = tables.goatnets, tables.components
    member_of = resolve.validate(goatnets, components)
    for warning in resolve.redundant_member_deviations(goatnets, components, member_of):
        print(warning)

    failed = False
    for member, goatnet in sorted(member_of.items()):
        text = render(member, goatnet, goatnets, components)
        path = os.path.join(GROUP_VARS, member, GENERATED_BASENAME)
        relpath = rel(path)
        if args.check:
            if os.path.exists(path):
                with open(path) as source_file:
                    current = source_file.read()
            else:
                current = ""
            if current != text:
                failed = True
                print("FAIL: %s is out of date with ops/pins/fleet-pins.yaml" % relpath)
                for line in difflib.unified_diff(current.splitlines(), text.splitlines(),
                                                 fromfile=relpath + " (committed)",
                                                 tofile=relpath + " (rendered)", lineterm=""):
                    print("  " + line)
        else:
            with open(path, "w") as f:
                f.write(text)
            print("wrote %s" % relpath)

    # -- second rendering: the IntentV1 artifact per goatnet ------------------
    caps = resolve.load_caps()
    allow, allow_len = resolve.load_allowlist()
    baseline = resolve.load_baseline()
    outside = resolve.scan_outside_pins(member_of)
    for goatnet in sorted(goatnets):
        path = intent_path(goatnet)
        rp = rel(path)
        committed = intent_mod.read_committed(path)
        body = intent_mod.build_body(goatnet, goatnets, components, caps,
                                     member_of, allow, outside,
                                     lite_goatnets=tables.lite_goatnets,
                                     table=tables.table, lite_table=tables.lite_table)
        text, doc = intent_mod.render(body, committed)
        intent_mod.validate_schema(doc, "rendered intent for '%s'" % goatnet)
        if args.check:
            if committed:
                intent_mod.validate_schema(committed, "committed %s" % rp)
                # The §3.4 integrity story, all three layers: the offline-CA
                # signature (WHO produced the bytes), content_hash
                # self-consistency (the body is what it claims), and the
                # regenerate-diff below (the tree still agrees).
                stated, actual = intent_mod.self_consistent(committed)
                if stated != actual:
                    failed = True
                    print("FAIL: %s carries content_hash %s but its body hashes to %s "
                          "— the file was hand-edited." % (rp, stated, actual))
                # CF-5 signature gate: an artifact whose signature is missing
                # or does not verify FAILS. Fail-closed on `unavailable` too —
                # a gate that cannot verify must not report green.
                v = signing.verify_file(path, doc=committed)
                if v.state != "verified":
                    failed = True
                    print("FAIL: %s" % v.detail)
            if os.path.exists(path):
                with open(path) as source_file:
                    current = source_file.read()
            else:
                current = ""
            if current != text:
                failed = True
                print("FAIL: %s is out of date with the tree." % rp)
                for line in difflib.unified_diff(current.splitlines(), text.splitlines(),
                                                 fromfile=rp + " (committed)",
                                                 tofile=rp + " (rendered)", lineterm=""):
                    print("  " + line)
        else:
            os.makedirs(os.path.dirname(path), exist_ok=True)
            if os.path.exists(path):
                with open(path) as source_file:
                    current = source_file.read()
            else:
                current = ""
            with open(path, "w") as f:
                f.write(text)
            print("wrote %s (serial %d, %s)" % (rp, doc["serial"], doc["content_hash"]))
            if current != text:
                # The generate-mesh-hosts.py rule: the generator regenerates,
                # the OPERATOR holding the goatnet's offline-CA age key signs.
                print("      body changed — the committed signature is now STALE; "
                      "%s" % signing.SIGNER_HINT)

    if not args.check:
        return 0

    gov = {gn: resolve.governed_vars_for(components, gn) for gn in goatnets}
    for member, key, path in outside:
        row = "%s:%s" % (member, key)
        if key in gov.get(member_of.get(member), set()):
            failed = True
            print("FAIL: %s defines %s (%s), which is GOVERNED by "
                  "ops/pins/fleet-pins.yaml." % (path, key, member))
            print("      vars.yml sorts after pins.yml, so this hand pin silently "
                  "shadows the generated one. Delete it.")
        elif row not in allow:
            failed = True
            print("FAIL: %s defines %s (%s) outside the generated pins.yml." % (path, key, member))
            print("      Migrate it into ops/pins/fleet-pins.yaml, or add "
                  "'%s' to ops/pins/unmigrated-pins.txt" % row)
            print("      and raise its `# baseline:` line (a visible, reviewable regression).")

    if baseline is None:
        failed = True
        print("FAIL: ops/pins/unmigrated-pins.txt declares no `# baseline: <n>` line — "
              "the shrink-only ratchet has no number to hold.")
        baseline = 0
    if allow_len > baseline:
        failed = True
        print("FAIL: ops/pins/unmigrated-pins.txt has %d rows; baseline is %d "
              "(it may only shrink)." % (allow_len, baseline))

    stale = sorted(r for r in allow
                   if r not in {"%s:%s" % (m, k) for m, k, _ in outside})
    if stale:
        failed = True
        print("FAIL: ops/pins/unmigrated-pins.txt lists rows that no longer exist: %s"
              % ", ".join(stale))
        print("      Delete them and lower the `# baseline:` line — the ratchet "
              "only pays off if it tightens.")

    if failed:
        print()
        print("Canon: docs/design/environment-parity.md §3. Regenerate with "
              "`goatbuilder pins` or `python3 ops/pins/generate-pins.py` (both avenues, ADR 1201).")
        return 1

    print("ok: fleet pins — %d component(s) across %d member group(s); "
          "%d/%d unmigrated pins allowlisted (shrinking); intent current for %s."
          % (len(components), len(member_of), allow_len, baseline,
             ", ".join(sorted(goatnets))))
    return 0


def check_goatnet(goatnet):
    """One goatnet's part of --check, as deploy-component.sh runs it before any
    host: its members' pins.yml are what the table renders, and its committed
    intent is there, its content_hash is its body's, its detached signature
    verifies, and it is what the tree renders. No network. Prints one FAIL line
    per reason, in the words of goatbuilder's pins::check_goatnet."""
    why = []
    detail = ""
    try:
        detail = _check_goatnet_into(goatnet, why)
    except (SchemaError, GrammarError, OSError) as exc:
        why.append(str(exc))
    for line in why:
        print("FAIL: %s" % line)
    if not why:
        print("ok: %s" % detail)
    return 1 if why else 0


def _check_goatnet_into(goatnet, why):
    tables = resolve.load_tables()
    goatnets, components = tables.goatnets, tables.components
    member_of = resolve.validate(goatnets, components)
    if goatnet not in goatnets:
        raise SchemaError("unknown goatnet %r" % goatnet)
    for member in sorted(m for m, g in member_of.items() if g == goatnet):
        path = os.path.join(GROUP_VARS, member, GENERATED_BASENAME)
        current = ""
        if os.path.exists(path):
            with open(path, encoding="utf-8") as f:
                current = f.read()
        if current != render(member, goatnet, goatnets, components):
            why.append("%s is out of date with ops/pins/fleet-pins.yaml" % rel(path))
    path = intent_path(goatnet)
    rp = rel(path)
    committed = intent_mod.read_committed(path)
    if not committed:
        why.append("%s is not there, or is not a JSON object: goatnet %s has no signed intent" % (rp, goatnet))
        return ""
    stated, actual = intent_mod.self_consistent(committed)
    if stated != actual:
        why.append("%s carries content_hash %s but its body hashes to %s "
                   "— the file was hand-edited." % (rp, stated, actual))
    v = signing.verify_file(path, doc=committed)
    if v.state != "verified":
        why.append(v.detail)
    intent_mod.validate_schema(committed, "committed %s" % rp)
    caps = resolve.load_caps()
    allow, _ = resolve.load_allowlist()
    outside = resolve.scan_outside_pins(member_of)
    body = intent_mod.build_body(goatnet, goatnets, components, caps, member_of, allow, outside,
                                 lite_goatnets=tables.lite_goatnets,
                                 table=tables.table, lite_table=tables.lite_table)
    text, _doc = intent_mod.render(body, committed)
    with open(path, encoding="utf-8") as f:
        if f.read() != text:
            why.append("%s is out of date with the tree." % rp)
    return v.detail


def main():
    ap = argparse.ArgumentParser(description="Render group_vars/<member>/pins.yml and "
                                             "ops/consistency/intent/<goatnet>/intent.json "
                                             "from ops/pins/fleet-pins.yaml")
    ap.add_argument("--check", action="store_true",
                    help="validate + diff instead of writing; exit 1 on drift")
    ap.add_argument("--goatnet", default="",
                    help="with --check: this goatnet's pins.yml files and signed intent only, "
                         "as a deploy verifies them (deploy-component.sh, GB-383)")
    args = ap.parse_args()
    if args.goatnet:
        if not args.check:
            ap.error("--goatnet is for --check")
        return check_goatnet(args.goatnet)
    try:
        return run(args)
    except (SchemaError, GrammarError) as exc:
        sys.stderr.write("ERROR: %s\n" % exc)
        return 2


if __name__ == "__main__":
    sys.exit(main())
