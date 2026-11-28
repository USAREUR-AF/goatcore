"""Compare a router render against the pre-rr3p (main) render, allowing only
the differences GHSA-rr3p intends, for the guards that assert a default render
is unchanged (founder/render-test.py, partner-partition/render-test.sh).

The intended differences, all in per-uuid-slot-pool mode:
  - the slot-<anchor> subjects come from the slot fragment between the
    `BEGIN: slot subjects` / `END: slot subjects` markers; with no fragment
    each names "<anchor>.unassigned" instead of the bare anchor;
  - trial-peer-read is an enumerated cert_common_names list (never
    link_protocols, never a slot CN);
  - one added policy binds the universal read to the slot subjects;
  - comments.
equivalent() normalizes exactly those and requires everything else to match;
intended() checks the rr3p side carries them.
"""
import re

_SLOT_SUBJECT = re.compile(r'(id: "slot-([0-9a-f]{32})",\n\s*cert_common_names: )\[[^\]]*\]')
_TPR = re.compile(r'\{\s*id: "trial-peer-read",.*?\n\s*\},', re.S)
_SLOT_READ_POLICY = re.compile(r'^\s*\{ rules: \[[^\]]*"audit-operator-read-deny"[^\]]*\], subjects: \["slot-[^\]]*\] \},\n', re.M)


def _strip_comments(text):
    out = []
    for line in text.splitlines():
        if line.strip().startswith("//"):
            continue
        line = re.sub(r"\s+//.*$", "", line)  # trailing comment (no // inside the strings we render)
        if line.strip():
            out.append(line.rstrip())
    return "\n".join(out) + "\n"


def _is_migration(main_render, rr3p_render):
    """Whether this is the one old-render/new-render comparison we permit."""
    main_tpr = _TPR.search(main_render)
    rr3p_tpr = _TPR.search(rr3p_render)
    return bool(
        main_tpr
        and rr3p_tpr
        and "link_protocols" in main_tpr.group(0)
        and "cert_common_names" in rr3p_tpr.group(0)
        and not _SLOT_READ_POLICY.search(main_render)
        and _SLOT_READ_POLICY.search(rr3p_render)
    )


def _normalize(text, migration, rr3p):
    t = _strip_comments(text)
    if migration:
        t = _SLOT_SUBJECT.sub(r"\1[SLOT-CN]", t)
        t = _TPR.sub('{ id: "trial-peer-read", TPR },', t)
        if rr3p:
            t = _SLOT_READ_POLICY.sub("", t)
    return t


def equivalent(main_render, rr3p_render):
    """True when the two renders differ only by the rr3p-intended changes."""
    migration = _is_migration(main_render, rr3p_render)
    return _normalize(main_render, migration, False) == _normalize(rr3p_render, migration, True)


def first_difference(main_render, rr3p_render, context=3):
    import difflib
    migration = _is_migration(main_render, rr3p_render)
    a = _normalize(main_render, migration, False).splitlines()
    b = _normalize(rr3p_render, migration, True).splitlines()
    return list(difflib.unified_diff(a, b, lineterm="", n=context))[:40]


def comparison_self_test(current_render):
    """Exercise the current/current comparison's fail-closed boundaries."""
    problems = []
    if not equivalent(current_render, current_render):
        problems.append("identical current renders differ")

    policy = _SLOT_READ_POLICY.search(current_render)
    if not policy:
        problems.append("current render has no slot-read policy to exercise")
    else:
        without_policy = _SLOT_READ_POLICY.sub("", current_render)
        if equivalent(current_render, without_policy):
            problems.append("removing the slot-read policy from the right render compares equal")
        if equivalent(without_policy, current_render):
            problems.append("removing the slot-read policy from the left render compares equal")
        policy_line = policy.group(0).strip()
        if policy_line not in "\n".join(first_difference(current_render, without_policy)):
            problems.append("removing the slot-read policy from the right render produces no useful diff")
        if policy_line not in "\n".join(first_difference(without_policy, current_render)):
            problems.append("removing the slot-read policy from the left render produces no useful diff")

    slot = re.search(r'id: "slot-([0-9a-f]{32})"', current_render)
    if not slot:
        problems.append("current render has no slot identity to exercise")
    else:
        changed_anchor = ("0" if slot.group(1)[0] != "0" else "1") + slot.group(1)[1:]
        changed_identity = current_render[:slot.start(1)] + changed_anchor + current_render[slot.end(1):]
        if equivalent(current_render, changed_identity):
            problems.append("changing a slot identity compares equal")
        if changed_anchor not in "\n".join(first_difference(current_render, changed_identity)):
            problems.append("changing a slot identity produces no useful diff")

    rule = 'id: "trial-peer-r-all"'
    if rule not in current_render:
        problems.append("current render has no unrelated rule to exercise")
    else:
        changed_rule = current_render.replace(rule, 'id: "trial-peer-r-all-changed"', 1)
        if equivalent(current_render, changed_rule):
            problems.append("changing an unrelated rule compares equal")
        if "trial-peer-r-all-changed" not in "\n".join(first_difference(current_render, changed_rule)):
            problems.append("changing an unrelated rule produces no useful diff")
    return problems


def intended(rr3p_render):
    """Problems with the rr3p side of a per-uuid-slot-pool render (empty = fine)."""
    problems = []
    if rr3p_render.count("// BEGIN: slot subjects") != 1 or rr3p_render.count("// END: slot subjects") != 1:
        problems.append("expected exactly one slot-subject region")
    for anchor_cn in re.findall(r'id: "slot-([0-9a-f]{32})",\n\s*cert_common_names: \[([^\]]*)\]', rr3p_render):
        anchor, cns = anchor_cn
        if not re.fullmatch(r'"%s(\.g[1-9][0-9]*|\.unassigned)?"' % anchor, cns.strip()):
            problems.append(f"slot-{anchor} names {cns}")
    tpr = _TPR.search(rr3p_render)
    if not tpr or "link_protocols" in tpr.group(0) or re.search(r'"[0-9a-f]{32}', tpr.group(0)):
        problems.append("trial-peer-read is not an enumerated, slot-free list")
    if re.search(r"cert_common_names\s*:\s*\[\s*\]", rr3p_render):
        problems.append("an empty cert_common_names list (zenoh 1.9 refuses to start on one)")
    return problems
