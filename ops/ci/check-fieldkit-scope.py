#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""check-fieldkit-scope.py — a Fieldkit tool can do nothing the goat CLI cannot.

THE RULE (docs/design/fieldkit-and-bleating-edge.md §1, §3, §5). A Fieldkit tool
is a small page over the same API the `goat` CLI calls: a fixed list of at most
eight actions, each equal to a CLI command, and no authority of its own. This
guard fails the build when a tool gains an action with no CLI equivalent.

WHAT IT CHECKS, for every tool page ops/fieldkit/<tool>/*.html (the design
system, its starter template, the e2e suite and node_modules are not tools):
  1. every element with `data-action` also has `data-cli` (the goat verb) and
     `data-api` ("METHOD /route"), and that (tool, verb, method, route) is a row
     in ops/ci/fieldkit-cli-allowlist.txt;
  2. a tool declares at most eight actions;
  3. outside the inlined Bleating Edge script, the page has no other way to
     reach a network: no fetch, XMLHttpRequest, WebSocket, EventSource,
     sendBeacon or dynamic import, and no API route written anywhere except in
     a `data-api` attribute. Bleating Edge's BE.call reaches only routes that an
     action declares, so (1) covers every request the page can make;
  4. with --docs FILE (the output of `goat docs --markdown`), every verb on the
     allow list exists in the CLI.

Exit 0 when every tool is in scope, 1 when one is not (each problem is named).

Usage:
  ops/ci/check-fieldkit-scope.py [--root DIR] [--allowlist FILE] [--docs FILE]
  ops/ci/check-fieldkit-scope.py --self-test
"""
from __future__ import annotations

import argparse
import re
import sys
import tempfile
from html.parser import HTMLParser
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
MAX_ACTIONS = 8
NOT_TOOLS = {"bleating-edge", "e2e", "node_modules"}
BE_SCRIPT = re.compile(r"<script data-bleating-edge>.*?</script>", re.S)
DATA_API_ATTR = re.compile(r"""data-api\s*=\s*("[^"]*"|'[^']*')""")
NETWORK = re.compile(r"\bfetch\s*\(|XMLHttpRequest|\bWebSocket\b|\bEventSource\b|sendBeacon|\bimport\s*\(")
ROUTE = re.compile(r"""["'`]\s*/(v\d+|api)/""")


def load_allowlist(path: Path) -> set[tuple[str, str, str, str]]:
    rows = set()
    for n, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        line = line.split("#", 1)[0].strip()
        if not line:
            continue
        parts = [p.strip() for p in line.split("|")]
        if len(parts) != 4 or not all(parts):
            raise SystemExit(f"{path}:{n}: expected 'tool | verb | METHOD | /route'")
        tool, verb, method, route = parts
        rows.add((tool, " ".join(verb.split()), method.upper(), route))
    return rows


class Actions(HTMLParser):
    def __init__(self) -> None:
        super().__init__(convert_charrefs=True)
        self.found: list[tuple[int, dict]] = []

    def handle_starttag(self, tag, attrs):
        a = dict(attrs)
        if "data-action" in a or "data-api" in a or "data-cli" in a:
            self.found.append((self.getpos()[0], a))

    handle_startendtag = handle_starttag


def tool_pages(root: Path) -> list[Path]:
    base = root / "ops" / "fieldkit"
    if not base.is_dir():
        return []
    return sorted(p for p in base.rglob("*.html")
                  if p.relative_to(base).parts[0] not in NOT_TOOLS and not NOT_TOOLS & set(p.parts))


def check_page(page: Path, tool: str, allow: set, rel: str) -> list[str]:
    problems = []
    text = page.read_text(encoding="utf-8")
    parser = Actions()
    parser.feed(text)
    names = set()
    for line, a in parser.found:
        where = f"{rel}:{line}"
        name = a.get("data-action")
        if not name:
            problems.append(f"{where}: data-api or data-cli without data-action; declare it as an action")
            continue
        names.add(name)
        verb = " ".join((a.get("data-cli") or "").split())
        api = (a.get("data-api") or "").split()
        if not verb:
            problems.append(f"{where}: action '{name}' has no data-cli; every action names the goat command it equals")
            continue
        if len(api) != 2:
            problems.append(f"{where}: action '{name}' needs data-api=\"METHOD /route\"")
            continue
        row = (tool, verb, api[0].upper(), api[1])
        if row not in allow:
            problems.append(f"{where}: action '{name}' = `goat {verb}` {api[0]} {api[1]} is not on "
                            f"ops/ci/fieldkit-cli-allowlist.txt for tool '{tool}'. A tool may only do what a CLI verb does")
    if len(names) > MAX_ACTIONS:
        problems.append(f"{rel}: {len(names)} actions; a Fieldkit tool has at most {MAX_ACTIONS}")
    rest = DATA_API_ATTR.sub("", BE_SCRIPT.sub("", text))
    for n, line in enumerate(rest.splitlines(), 1):
        if NETWORK.search(line):
            problems.append(f"{rel}: line {n} of the page outside Bleating Edge reaches the network directly; "
                            f"call a declared action with BE.call instead")
        if ROUTE.search(line):
            problems.append(f"{rel}: line {n} writes an API route outside a data-api attribute; "
                            f"declare it on an action instead")
    return problems


def check_docs(allow: set, docs: Path) -> list[str]:
    text = docs.read_text(encoding="utf-8")
    have = set(re.findall(r"^#+\s*`goat ([^`]+)`", text, re.M))
    have = {" ".join(h.split()) for h in have}
    return [f"allow list: `goat {verb}` (tool {tool}) is not a command in {docs}"
            for tool, verb, _, _ in sorted(allow) if verb not in have]


def run(root: Path, allowlist: Path, docs: Path | None) -> list[str]:
    allow = load_allowlist(allowlist)
    problems = []
    base = root / "ops" / "fieldkit"
    for page in tool_pages(root):
        tool = page.relative_to(base).parts[0]
        problems += check_page(page, tool, allow, str(page.relative_to(root)))
    if docs:
        problems += check_docs(allow, docs)
    return problems


GOOD = """<nav>
<button data-action="status" data-cli="pod status" data-api="GET /v1/status">Status</button>
</nav>
<script data-bleating-edge>function call(){ return fetch("/v1/" + x); }</script>
<script>BE.call("status");</script>"""


def self_test() -> int:
    allow_text = "t | pod status | GET | /v1/status\n"
    cases = [
        ("in scope", GOOD, 0),
        ("verb not on the list", GOOD.replace("pod status", "pod nuke"), 1),
        ("route not on the list", GOOD.replace("GET /v1/status", "POST /v1/status"), 1),
        ("no data-cli", GOOD.replace(' data-cli="pod status"', ""), 1),
        ("no data-api", GOOD.replace(' data-api="GET /v1/status"', ""), 1),
        ("data-api without action", GOOD + '<a data-api="GET /v1/status">x</a>', 1),
        ("direct fetch", GOOD + "<script>fetch('/v1/rules', {method: 'DELETE'})</script>", 1),
        ("route literal", GOOD + "<script>var r = \"/v1/rules\";</script>", 1),
        ("xhr", GOOD + "<script>new XMLHttpRequest()</script>", 1),
        ("nine actions", "".join(f'<b data-action="a{i}" data-cli="pod status" data-api="GET /v1/status"></b>'
                                 for i in range(9)), 1),
    ]
    failed = 0
    with tempfile.TemporaryDirectory() as d:
        root = Path(d)
        (root / "ops/fieldkit/t").mkdir(parents=True)
        (root / "ops/fieldkit/bleating-edge").mkdir(parents=True)
        (root / "ops/fieldkit/bleating-edge/starter.html").write_text('<b data-action="x"></b>')
        allow = root / "allow.txt"
        allow.write_text(allow_text)
        for name, html, want in cases:
            (root / "ops/fieldkit/t/t.html").write_text(html)
            got = 1 if run(root, allow, None) else 0
            if got != want:
                failed += 1
                print(f"FAIL {name}: want exit {want}, got {got}: {run(root, allow, None)}")
        docs = root / "docs.md"
        (root / "ops/fieldkit/t/t.html").write_text(GOOD)
        docs.write_text("## `goat pod status`\n")
        if run(root, allow, docs):
            failed += 1
            print("FAIL docs: a listed verb present in the CLI was refused")
        docs.write_text("## `goat pod rules ls`\n")
        if not run(root, allow, docs):
            failed += 1
            print("FAIL docs: a listed verb missing from the CLI was accepted")
    print(f"check-fieldkit-scope self-test: {len(cases) + 2 - failed}/{len(cases) + 2} passed")
    return 1 if failed else 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--root", type=Path, default=REPO)
    ap.add_argument("--allowlist", type=Path, default=None)
    ap.add_argument("--docs", type=Path, default=None, help="output of `goat docs --markdown`")
    ap.add_argument("--self-test", action="store_true")
    a = ap.parse_args()
    if a.self_test:
        return self_test()
    allowlist = a.allowlist or a.root / "ops" / "ci" / "fieldkit-cli-allowlist.txt"
    problems = run(a.root, allowlist, a.docs)
    for p in problems:
        print(f"FAIL {p}")
    pages = tool_pages(a.root)
    if not problems:
        print(f"check-fieldkit-scope: {len(pages)} tool page(s); every action equals an allowed goat command")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
