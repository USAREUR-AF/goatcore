#!/usr/bin/env python3
"""Every oauth2-proxy fence refuses a cookie secret oauth2-proxy cannot use, before deploying.

oauth2-proxy uses OAUTH2_PROXY_COOKIE_SECRET as URL-safe base64 when it decodes
to 16, 24 or 32 bytes, and otherwise as raw text that must be exactly 16, 24 or
32 characters. Anything else (standard base64 with + or /, a generated
password) crash-loops the proxy at boot while the play recap is green and the
surface behind it answers 502. A role that renders the secret must therefore
assert the exact length set before it deploys (a role may also accept the
URL-safe base64 form); a non-empty or minimum-length check does not catch it.

A fence is a role template line OAUTH2_PROXY_COOKIE_SECRET={{ <var> }}. It
passes when a task file of the same role asserts `(<var> | length) in [16, 24, 32]`
(filters before `length`, such as `trim`, are allowed).

    python3 ops/ci/check-fence-cookie-secret-length.py           # exit 1 on a gap
    python3 ops/ci/check-fence-cookie-secret-length.py --self-test
"""
from __future__ import annotations

import re
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
TEMPLATE_VAR = re.compile(r"OAUTH2_PROXY_COOKIE_SECRET=\{\{\s*([a-z0-9_]+)")


def exact_set_check(var: str) -> re.Pattern:
    # `(var | length)` or with filters before length, e.g. `(var | trim | length)`.
    return re.compile(rf"\(\s*{re.escape(var)}\s*(?:\|\s*\w+\s*)*\|\s*length\s*\)\s*in\s*\[\s*16\s*,\s*24\s*,\s*32\s*\]")


def gaps(ansible: Path):
    out = []
    for tpl in sorted(ansible.glob("roles/*/templates/*.j2")):
        role = tpl.parents[1]
        tasks = "\n".join(f.read_text(errors="replace") for f in sorted((role / "tasks").glob("*.yml")))
        for var in sorted(set(TEMPLATE_VAR.findall(tpl.read_text(errors="replace")))):
            if not exact_set_check(var).search(tasks):
                out.append((var, tpl.relative_to(ansible.parent.parent)))
    return out


def self_test() -> int:
    with tempfile.TemporaryDirectory() as d:
        a = Path(d) / "ops" / "ansible"
        for role, check in (("good", "- (good_cookie | length) in [16, 24, 32]"),
                            ("floor", "- floor_cookie | length > 0"),
                            ("none", "")):
            (a / "roles" / role / "templates").mkdir(parents=True)
            (a / "roles" / role / "tasks").mkdir(parents=True)
            (a / "roles" / role / "templates" / "p.env.j2").write_text(
                f"OAUTH2_PROXY_COOKIE_SECRET={{{{ {role}_cookie }}}}\n")
            (a / "roles" / role / "tasks" / "main.yml").write_text(
                f"- ansible.builtin.assert:\n    that:\n      {check}\n" if check else "- debug: {}\n")
        got = [g[0] for g in gaps(a)]
    want = ["floor_cookie", "none_cookie"]
    ok = got == want
    print(f"self-test: {'ok' if ok else 'FAIL'} (gaps={got}, want {want})")
    return 0 if ok else 1


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    found = gaps(ROOT / "ops" / "ansible")
    if not found:
        print("ok: every oauth2-proxy fence asserts its cookie secret is 16, 24 or 32 characters")
        return 0
    for var, tpl in found:
        print(f"FAIL: {tpl} renders {{{{ {var} }}}} as OAUTH2_PROXY_COOKIE_SECRET and no task in its role "
              f"asserts `({var} | length) in [16, 24, 32]`. A secret oauth2-proxy cannot use crash-loops it "
              "behind a green play (roles/microscope/tasks/main.yml is the shape).")
    return 1


if __name__ == "__main__":
    sys.exit(main())
