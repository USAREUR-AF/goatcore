#!/usr/bin/env python3
"""A numeric column's HEADER must align the same way as its DATA.

WHY THIS EXISTS. Every operator surface in this repo styles numeric cells with
`td.n{text-align:right}` and headers with a blanket `th{text-align:left}`. The
markup was always correct — the tables carry `<th class="n">` — but no selector
matched `th.n`, so the header of every numeric column sat left-aligned above
right-aligned numbers. The gap scales with column width, which is why it looks
acceptable on a narrow column like REQ/S and obviously broken on a wide one like
NAMED CLIENTS, where the label sat about 115px left of its own digits.

Three files carried it identically (peerview.css, netview.html, response.html)
because the stylesheet was copy-pasted, and `panoscope/pages.css` did
NOT, because it happened to write `.data-table th.num, .data-table td.num` as a
single rule. That is the pattern this guard enforces: alignment is a property of
the COLUMN, so both cell types are set together and cannot drift apart.

It is a cascade bug, not a markup bug, which is why review kept missing it — the
HTML reads correctly on its own and the CSS reads correctly on its own.

usage:
  ops/ci/check-table-header-alignment.py            # scan, exit 1 on a finding
"""
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
TARGETS = ["ops/*/www/*.html", "ops/*/www/*.css",
           "ops/*/frontend/src/*.css", "ops/*/*/www/*.html"]

# `td.CLASS{...text-align:right...}` — the data side of a numeric column.
TD_RIGHT = re.compile(
    r"(?<![\w.-])td\.([\w-]+)\s*(?:,[^{]*)?\{[^}]*text-align\s*:\s*right", re.S)
# any selector that puts th.CLASS on the right
TH_RIGHT = re.compile(
    r"(?<![\w.-])th\.([\w-]+)[^{]*\{[^}]*text-align\s*:\s*right", re.S)
# a header actually using the class in markup
TH_MARKUP = re.compile(r"<th[^>]*\bclass\s*=\s*[\"']([^\"']+)[\"']")
# CSS and HTML comments, stripped before matching. Not cosmetic: the FIRST
# version of this guard passed its own mutation test because the explanatory
# comment above the fix mentions `th.n`, and TH_RIGHT's `[^{]*` then scanned
# forward out of the comment into the next real rule containing
# text-align:right. The guard read its own prose as evidence the rule existed.
# A guard that can be satisfied by a comment about the guard is not a guard.
COMMENTS = re.compile(r"/\*.*?\*/|<!--.*?-->", re.S)


def scan(root):
    root = pathlib.Path(root)
    findings = []
    seen = set()
    for pat in TARGETS:
        for path in sorted(root.glob(pat)):
            if path in seen or not path.is_file():
                continue
            seen.add(path)
            text = COMMENTS.sub(" ", path.read_text(encoding="utf-8", errors="replace"))
            right_td = set(TD_RIGHT.findall(text))
            right_th = set(TH_RIGHT.findall(text))
            markup_th = {c for m in TH_MARKUP.findall(text) for c in m.split()}
            for cls in sorted(right_td - right_th):
                # Only a finding when a header actually uses the class. A data-only
                # class with no matching header is not misaligned, it just has a
                # header styled some other way.
                if cls in markup_th:
                    findings.append((path.relative_to(root), cls))
    return findings


def main():
    findings = scan(ROOT)

    if not findings:
        print("OK — every numeric column header aligns with its data")
        return 0

    print("MISALIGNED COLUMN HEADERS\n")
    for path, cls in findings:
        print("  %s" % path)
        print("      <th class=\"%s\"> exists and td.%s is right-aligned, but no"
              % (cls, cls))
        print("      selector right-aligns th.%s — so the header falls back to the" % cls)
        print("      blanket th{text-align:left} and sits left of its own numbers.")
        print("      Fix: set both in one rule —  td.%s,th.%s{text-align:right}\n"
              % (cls, cls))
    print("%d column(s) affected. Alignment is a property of the COLUMN: set it"
          % len(findings))
    print("on td and th in the SAME rule so the two cannot drift apart.")
    return 1


if __name__ == "__main__":
    sys.exit(main())
