#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""inline.py — copy Bleating Edge into each Fieldkit tool, so a tool is one file.

A Fieldkit tool ships as one HTML file with no outside assets
(docs/design/fieldkit-and-bleating-edge.md §3). The design system lives once,
in ops/fieldkit/bleating-edge/bleating-edge.{css,js}. This script writes those
two files into every tool between its markers:

    <style data-bleating-edge>  ...  </style>
    <script data-bleating-edge> ...  </script>

The committed tool file already carries the copy, so serving it needs no build.
CI runs `--check`, which fails when a tool's copy differs from the source.

Usage:
  ops/fieldkit/inline.py            # rewrite every tool in place
  ops/fieldkit/inline.py --check    # exit 1 when any tool is stale
  ops/fieldkit/inline.py FILE...    # only these files
"""
from __future__ import annotations

import re
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
CSS = HERE / "bleating-edge" / "bleating-edge.css"
JS = HERE / "bleating-edge" / "bleating-edge.js"

STYLE_RE = re.compile(r"(<style data-bleating-edge>)(.*?)(</style>)", re.S)
SCRIPT_RE = re.compile(r"(<script data-bleating-edge>)(.*?)(</script>)", re.S)


def tools() -> list[Path]:
    """Every HTML file under ops/fieldkit that carries a marker."""
    out = []
    for p in sorted(HERE.rglob("*.html")):
        if "node_modules" in p.parts:
            continue
        text = p.read_text(encoding="utf-8")
        if STYLE_RE.search(text) or SCRIPT_RE.search(text):
            out.append(p)
    return out


def render(text: str, css: str, js: str) -> str:
    if not STYLE_RE.search(text) or not SCRIPT_RE.search(text):
        raise ValueError("needs both <style data-bleating-edge> and <script data-bleating-edge> markers")
    text = STYLE_RE.sub(lambda m: m.group(1) + "\n" + css + m.group(3), text, count=1)
    text = SCRIPT_RE.sub(lambda m: m.group(1) + "\n" + js + m.group(3), text, count=1)
    return text


def main(argv: list[str]) -> int:
    check = "--check" in argv
    files = [Path(a).resolve() for a in argv if not a.startswith("--")] or tools()
    css = CSS.read_text(encoding="utf-8")
    js = JS.read_text(encoding="utf-8")
    if "</style" in css or "</script" in js:
        print("inline.py: the design system source must not contain a closing style or script tag", file=sys.stderr)
        return 2
    stale = []
    for p in files:
        text = p.read_text(encoding="utf-8")
        try:
            new = render(text, css, js)
        except ValueError as e:
            print(f"inline.py: {p}: {e}", file=sys.stderr)
            return 2
        if new != text:
            stale.append(p)
            if not check:
                p.write_text(new, encoding="utf-8")
    rel = lambda p: p.relative_to(HERE.parents[1]) if p.is_relative_to(HERE.parents[1]) else p  # noqa: E731
    if check and stale:
        for p in stale:
            print(f"STALE {rel(p)}: its Bleating Edge copy differs from ops/fieldkit/bleating-edge/. "
                  f"Fix: python3 ops/fieldkit/inline.py", file=sys.stderr)
        return 1
    for p in stale:
        print(f"updated {rel(p)}")
    if check:
        print(f"inline.py: {len(files)} tool file(s) carry the current Bleating Edge")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
