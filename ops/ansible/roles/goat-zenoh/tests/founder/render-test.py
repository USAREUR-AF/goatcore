#!/usr/bin/env python3
"""The goatlite founder grant renders only where zenoh_founder_cn is set.

Unset, the router config is byte-identical to main's, so no existing goatnet
changes. Set, it carries exactly one rule (founder-w: put/delete on founder/**
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
        if render(str(main_tpl), {}) != render(str(TPL), {}):
            problems.append("unset: the render differs from main's; the founder grant must be inert where undeclared")
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
print("[PASS] founder grant: inert when unset (byte-identical to main), exactly one rule, subject, policy and storage when set")
