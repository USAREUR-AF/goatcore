#!/usr/bin/env python3
"""The goatlite founder grant renders only where zenoh_founder_cn is set.

Unset, the router config matches main's apart from the changes GHSA-rr3p makes
to every slot-pool render (slot-fragment/rr3p_render_diff.py), so no existing
goatnet changes. Set, it carries exactly one rule (founder-w: put/delete on founder/**
and hello/<site_id>), one subject (the CN), one policy binding them with the
ordinary peer read, and one storage on founder/** so the founder's writes are
retained and `goat pub --confirm` can read them back. Renders with the partner-partition test's renderer.
"""
import importlib.util, subprocess, sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
TPL = HERE.parent.parent / "templates" / "config.json5.j2"
spec = importlib.util.spec_from_file_location("r", HERE.parent / "partner-partition" / "render.py")
# render.py runs its CLI at import time; load only its render() by exec'ing the head.
src = (HERE.parent / "partner-partition" / "render.py").read_text()
head = src.split("\nmode, mine, main = sys.argv")[0]
ns = {}
exec(compile(head, "render.py", "exec"), ns)
render = ns["render"]

import os
REF = os.environ.get("MAIN_REF", "origin/main")
main_src = subprocess.run(["git", "show", f"{REF}:ops/ansible/roles/goat-zenoh/templates/config.json5.j2"],
                          cwd=HERE, capture_output=True, text=True)
problems = []
if main_src.returncode == 0:
    main_tpl = HERE / ".main-config.json5.j2"
    main_tpl.write_text(main_src.stdout)
    try:
        # GHSA-rr3p changes the default slot-pool render on purpose; the guard
        # is that nothing else (in particular no founder grant) differs.
        sys.path.insert(0, str(HERE.parent / "slot-fragment"))
        import rr3p_render_diff
        main_r, mine_r = render(str(main_tpl), {}), render(str(TPL), {})
        if not rr3p_render_diff.equivalent(main_r, mine_r):
            problems.append("unset: the render differs from main's beyond the rr3p-intended changes; the founder grant must be inert where undeclared: "
                            + " | ".join(rr3p_render_diff.first_difference(main_r, mine_r)[:12]))
        problems += ["unset: " + p for p in rr3p_render_diff.intended(mine_r)]
        problems += ["comparison self-test: " + p for p in rr3p_render_diff.comparison_self_test(mine_r)]
    finally:
        main_tpl.unlink()
on = render(str(TPL), {"zenoh_founder_cn": "goat-founder-trial-peer", "site_id": "pasture"})
if on.count('id: "founder-w"') != 1: problems.append("set: expected one founder-w rule")
if '"founder/**", "hello/pasture"' not in on: problems.append("set: founder-w must write founder/** and hello/<site_id> only")
if on.count('cert_common_names: ["goat-founder-trial-peer"]') != 1: problems.append("set: expected one founder subject with the CN")
if '{ rules: ["trial-peer-r-all", "trial-peer-deliver-all", "founder-w"], subjects: ["founder"] }' not in on:
    problems.append("set: expected the founder policy with the ordinary peer read")
if on.count('key_expr: "founder/**"') != 1: problems.append("set: expected one storage on founder/**")
if on.count('/founder",') != 1: problems.append("set: expected the founder storage in its own rocksdb dir")
if problems:
    print("FAIL:", *problems, sep="\n  "); sys.exit(1)
print("[PASS] founder grant: inert when unset (identical to main apart from the rr3p changes), exactly one rule, subject, policy and storage when set")
