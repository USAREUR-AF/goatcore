#!/usr/bin/env python3
"""The error catalog is complete, cited codes exist, and the generated reference is current.

ops/errors/catalog.yaml is the one home for every error goat and goatbuilder print and every
tool error they carry (ADR 1199, docs/design/error-interpretation.md). This gate holds it to its
contract:

  * every entry has code, tool, title, what, at least one cause and one fix, and a read pointer
    that exists in the tree; codes are unique and shaped E_*, GB-NNN or X-<TOOL>-NNN
  * every `matches:` pattern compiles (Python `re`; the crate's own test compiles them with the
    Rust regex engine, which is the authoritative one)
  * every code a binary cites in Rust (`"GB-007"`, `"X-SSH-001"`) and every `E_*` code in
    crates/goat-cli/src/errors.rs exists in the catalog
  * every `E_*` row of docs/design/goat-cli-error-codes.md is in the catalog and vice versa,
    with the same exit code
  * docs/reference/errors.md equals what --write generates from the catalog
  * docs/reference/errors/<CODE>.md, one page per code, each equal what --write generates, and
    the directory holds no page for a code the catalog does not have

Exit 0 ok; 1 with every defect printed; 2 when an input cannot be read.

usage: check-error-catalog.py [--repo DIR] [--write] [--self-test]
"""
import argparse, os, re, sys, tempfile

try:
    import yaml
except ImportError:
    print("check-error-catalog: PyYAML missing (pip install pyyaml)", file=sys.stderr)
    sys.exit(2)

CATALOG = "ops/errors/catalog.yaml"
REFERENCE = "docs/reference/errors.md"
# One page per code, named by the code, so a search for a code finds a page whose
# title is that code. Generated from CATALOG, never edited by hand.
PAGES = "docs/reference/errors"
# The English Fluent catalog of the entries' prose — the arbiter the other
# locales are translated from (docs/design/goat-cli-i18n.md §"CLI catalog
# currency"). Generated from CATALOG, never edited by hand; the translations
# beside it are filled by ops/scripts/translate-ftl.py --catalog goat-errors.
FTL = "crates/goat-errors/i18n/en/errors.ftl"
CODES_DOC = "docs/design/goat-cli-error-codes.md"
GOAT_ERRORS_RS = "crates/goat-cli/src/errors.rs"
RUST_DIRS = ("crates/goatbuilder/src", "crates/goat-cli/src", "crates/goat-errors/src")
# A retired mark: the release tag that stopped raising the code (ADR 1228's form), then why.
RETIRED = re.compile(r"^(goatbuilder/v|goat-cli-v)\d+\.\d+\.\d+: .+")
CODE_SHAPE = re.compile(r"^(E_[A-Z][A-Z_]+|GB-\d{3}|X-[A-Z]+-\d{3})$")
CITED = re.compile(r'"((?:GB-\d{3}|X-[A-Z]+-\d{3}))"')
E_CODE_IN_RS = re.compile(r'#\[error\("(E_[A-Z_]+)"\)\]')
DOC_ROW = re.compile(r"^\|\s*`(E_[A-Z_]+)`\s*\|[^|]*\|\s*(\d+)\s*\|")
PREFIX_TITLE = {"E_": "goat", "GB-": "goatbuilder", "X-": "tools goat and goatbuilder call"}


def load(repo):
    with open(os.path.join(repo, CATALOG)) as f:
        data = yaml.safe_load(f)
    return data


def check_entries(repo, data, problems):
    if data.get("schema") != "goat-errors/v1":
        problems.append(f"{CATALOG}: schema must be goat-errors/v1")
    seen = set()
    for e in data.get("entries") or []:
        code = str(e.get("code", ""))
        where = f"{CATALOG}: {code or '<no code>'}"
        if not CODE_SHAPE.match(code):
            problems.append(f"{where}: code is not E_*, GB-NNN or X-<TOOL>-NNN")
        if code in seen:
            problems.append(f"{where}: duplicate code")
        seen.add(code)
        for k in ("tool", "title", "what", "read"):
            if not str(e.get(k, "")).strip():
                problems.append(f"{where}: missing {k}")
        for k in ("causes", "fixes"):
            if not isinstance(e.get(k), list) or not e[k]:
                problems.append(f"{where}: {k} must list at least one")
        if e.get("read") and not os.path.exists(os.path.join(repo, str(e["read"]))):
            problems.append(f"{where}: read points at {e['read']}, which is not in the tree")
        if code.startswith(("E_", "GB-")) and not isinstance(e.get("exit"), int):
            problems.append(f"{where}: a goat/goatbuilder code needs its exit code")
        if "retired" in e and not (isinstance(e["retired"], str) and RETIRED.match(e["retired"])):
            problems.append(f"{where}: retired must say \"<release tag>: <why>\", the tag of the release "
                            f"that stopped raising it (goatbuilder/vX.Y.Z or goat-cli-vX.Y.Z)")
        for m in e.get("matches") or []:
            try:
                re.compile(m)
            except re.error as err:
                problems.append(f"{where}: pattern {m!r} does not compile: {err}")
    return seen


def check_citations(repo, codes, problems):
    for d in RUST_DIRS:
        root = os.path.join(repo, d)
        if not os.path.isdir(root):
            continue
        for dirpath, _, files in os.walk(root):
            for fn in files:
                if not fn.endswith(".rs"):
                    continue
                p = os.path.join(dirpath, fn)
                with open(p) as f:
                    for n, line in enumerate(f, 1):
                        for c in CITED.findall(line):
                            if c not in codes:
                                problems.append(f"{os.path.relpath(p, repo)}:{n}: cites {c}, which is not in {CATALOG}")
    p = os.path.join(repo, GOAT_ERRORS_RS)
    if os.path.exists(p):
        with open(p) as f:
            for n, line in enumerate(f, 1):
                for c in E_CODE_IN_RS.findall(line):
                    if c not in codes:
                        problems.append(f"{GOAT_ERRORS_RS}:{n}: {c} is not in {CATALOG}")


def check_codes_doc(repo, data, codes, problems):
    p = os.path.join(repo, CODES_DOC)
    if not os.path.exists(p):
        return
    exits = {e["code"]: e.get("exit") for e in data.get("entries") or []}
    doc = {}
    with open(p) as f:
        for n, line in enumerate(f, 1):
            m = DOC_ROW.match(line)
            if m:
                doc[m.group(1)] = (int(m.group(2)), n)
    for c, (x, n) in doc.items():
        if c not in codes:
            problems.append(f"{CODES_DOC}:{n}: {c} is not in {CATALOG}")
        elif exits.get(c) != x:
            problems.append(f"{CODES_DOC}:{n}: {c} exit {x} differs from the catalog's {exits.get(c)}")
    for c in codes:
        if c.startswith("E_") and c not in doc:
            problems.append(f"{CODES_DOC}: no row for {c}, which {CATALOG} declares")


def render(data):
    out = []
    out.append("# Error reference — goat and goatbuilder\n")
    out.append("Generated from `ops/errors/catalog.yaml` by `ops/ci/check-error-catalog.py --write`; do not edit here. "
               "Look a code up with `goat explain <code>` or `goatbuilder explain <code>`; paste any error text to "
               "`goat explain -` / `goatbuilder explain -` to have it interpreted (ADR 1199, "
               "[error-interpretation.md](../design/error-interpretation.md)).\n")
    entries = data.get("entries") or []
    for prefix, title in PREFIX_TITLE.items():
        rows = [e for e in entries if str(e["code"]).startswith(prefix)]
        if not rows:
            continue
        out.append(f"\n## {title}\n")
        out.append("| Code | Tool | What it means | Why | What to do | Read | Exit |")
        out.append("|---|---|---|---|---|---|---|")
        for e in rows:
            cell = lambda xs: "<br>".join(str(x).replace("|", "\\|") for x in xs)
            out.append("| [`{code}`](errors/{code}.md) | {tool} | **{title}** — {what} | {causes} | {fixes} | `{read}` | {exit} |".format(
                code=e["code"], tool=e.get("tool", ""), title=str(e.get("title", "")).replace("|", "\\|"),
                what=str(e.get("what", "")).replace("|", "\\|"), causes=cell(e.get("causes") or []),
                fixes=cell(f"`{x}`" for x in (e.get("fixes") or [])), read=e.get("read", ""),
                exit=e.get("exit", "")))
    return "\n".join(out) + "\n"


def page_path(code):
    return f"{PAGES}/{code}.md"


def md_prose(value):
    """Catalog prose as Markdown text: outside `code spans`, a `<placeholder>` would
    be read as an HTML tag and dropped, and `*` would start emphasis."""
    parts = re.split(r"(`[^`]*`)", " ".join(str(value).split()))
    return "".join(p if p.startswith("`") else p.replace("&", "&amp;").replace("<", "&lt;").replace("*", "\\*")
                   for p in parts)


def render_page(e):
    """The page of one code: its title is the code and the entry's title, then
    what / why / fix / read. Links are relative to PAGES, three levels below the
    repository root."""
    code, tool = str(e["code"]), str(e.get("tool", ""))
    if code.startswith("X-"):
        who = f"`{code}` is an error of `{tool}` that goat and goatbuilder carry and interpret."
    else:
        who = f"`{code}` is printed by `{tool}`."
    out = [f"# {code}: {md_prose(e.get('title', ''))}", ""]
    out.append(f"Generated from `{CATALOG}` by `ops/ci/check-error-catalog.py --write`; do not edit here. "
               f"{who} The same text offline: `goat explain {code}` or `goatbuilder explain {code}`.")
    if e.get("retired"):
        out += ["", f"**Retired.** No current release raises this code ({md_prose(e['retired'])}). "
                    "The page stays because logs of earlier releases carry the code."]
    out += ["", "## What it means", "", md_prose(e.get("what", ""))]
    out += ["", "## Why it happens", ""]
    out += [f"- {md_prose(c)}" for c in e.get("causes") or []]
    out += ["", "## What to do", "", "One line per step or alternative:", "", "```text"]
    out += [" ".join(str(x).split()) for x in e.get("fixes") or []]
    out += ["```", "", "## Read more", "", f"- [`{e.get('read', '')}`](../../../{e.get('read', '')})",
            "- [Every error code](../errors.md)",
            "- [How errors are interpreted](../../design/error-interpretation.md)"]
    if "exit" in e:
        out += ["", "## Exit code", "", f"`{tool}` exits with {e['exit']}."]
    return "\n".join(out) + "\n"


def ftl_id(code):
    """The Fluent message id for an entry: lower case, `_` -> `-` (ids are
    [a-zA-Z][a-zA-Z0-9_-]*; every code already starts with a letter)."""
    return str(code).lower().replace("_", "-")


def ftl_text(value):
    """A catalog string as a Fluent value: one line, and the two characters
    Fluent reserves ({ and }) written as string literals so prose like
    `{ $var }` in a command stays literal text."""
    v = " ".join(str(value).split())
    return "".join('{"{"}' if ch == "{" else '{"}"}' if ch == "}" else ch for ch in v)


def render_ftl(data):
    """Only the prose localizes: title, what, each cause. Codes, fixes (the
    commands an operator types), read paths and exit codes never do."""
    out = ["### goat-errors — English (en), the arbiter",
           f"### Generated from {CATALOG} by ops/ci/check-error-catalog.py --write; do not edit.",
           "### Keys: <code>-title, <code>-what, <code>-cause-<n> (n from 1).", ""]
    for e in data.get("entries") or []:
        i = ftl_id(e["code"])
        out.append(f"{i}-title = {ftl_text(e.get('title', ''))}")
        out.append(f"{i}-what = {ftl_text(e.get('what', ''))}")
        for n, c in enumerate(e.get("causes") or [], 1):
            out.append(f"{i}-cause-{n} = {ftl_text(c)}")
    return "\n".join(out) + "\n"


def write(path, text):
    with open(path, "w") as f:
        f.write(text)


def self_test():
    with tempfile.TemporaryDirectory() as t:
        os.makedirs(os.path.join(t, "ops/errors"))
        os.makedirs(os.path.join(t, "docs/design"))
        os.makedirs(os.path.join(t, "docs/reference"))
        os.makedirs(os.path.join(t, "crates/goatbuilder/src"))
        write(os.path.join(t, "docs/design/x.md"), "x\n")
        good = {"schema": "goat-errors/v1", "entries": [
            {"code": "GB-001", "tool": "goatbuilder", "title": "t", "what": "w", "causes": ["c"], "fixes": ["f"], "read": "docs/design/x.md", "exit": 2, "matches": ["unknown component"]},
            {"code": "X-SSH-001", "tool": "ssh", "title": "t", "what": "w", "causes": ["c"], "fixes": ["f"], "read": "docs/design/x.md", "matches": ["Permission denied \\(publickey"]},
        ]}
        with open(os.path.join(t, CATALOG), "w") as f:
            yaml.safe_dump(good, f)
        write(os.path.join(t, "crates/goatbuilder/src/a.rs"), 'coded("GB-001", x)\n')
        write(os.path.join(t, REFERENCE), render(good))
        os.makedirs(os.path.join(t, PAGES))
        for e in good["entries"]:
            write(os.path.join(t, page_path(e["code"])), render_page(e))
        page = render_page(good["entries"][0])
        assert page.startswith("# GB-001: t\n"), "a page's title is the code and the entry's title"
        assert "goatbuilder explain GB-001" in page and "exits with 2" in page
        assert md_prose("no <site> in `a <b> *`, 2 * 3") == "no &lt;site> in `a <b> *`, 2 \\* 3"
        assert "**Retired.**" in render_page({**good["entries"][0], "retired": "goatbuilder/v0.6.0: gone"})
        os.makedirs(os.path.dirname(os.path.join(t, FTL)))
        write(os.path.join(t, FTL), render_ftl(good))
        assert run(t, write=False) == 0, "a complete catalog must pass"
        for mark, ok in (("goatbuilder/v0.6.0: standup reads the site from the plan", True),
                         ("goat-cli-v0.6.1: goat no longer reads profiles", True),
                         ("0.6.0: no tag", False), ("goatbuilder/v0.6: not a version", False),
                         ("goatbuilder/v0.6.0:", False), ("", False)):
            problems = []
            check_entries(t, {"schema": "goat-errors/v1", "entries": [{**good["entries"][0], "retired": mark}]}, problems)
            assert (not problems) == ok, f"retired {mark!r}: {problems}"
        write(os.path.join(t, page_path("GB-001")), "stale\n")
        assert run(t, write=False) == 1, "a stale code page must fail"
        write(os.path.join(t, page_path("GB-001")), page)
        write(os.path.join(t, page_path("GB-777")), "x\n")
        assert run(t, write=False) == 1, "a page for a code the catalog does not have must fail"
        os.remove(os.path.join(t, page_path("GB-777")))
        assert run(t, write=False) == 0
        assert ftl_text("run `x { $y }`") == 'run `x {"{"} $y {"}"}`', "braces are escaped as Fluent literals"
        assert "gb-001-cause-1 = c" in render_ftl(good), "causes are numbered from 1"
        write(os.path.join(t, FTL), "stale\n")
        assert run(t, write=False) == 1, "a stale English errors.ftl must fail"
        write(os.path.join(t, FTL), render_ftl(good))
        write(os.path.join(t, "crates/goatbuilder/src/a.rs"), 'coded("GB-009", x)\n')
        assert run(t, write=False) == 1, "a cited code missing from the catalog must fail"
        write(os.path.join(t, "crates/goatbuilder/src/a.rs"), 'coded("GB-001", x)\n')
        bad = dict(good)
        bad["entries"] = good["entries"] + [{"code": "GB-002", "tool": "goatbuilder", "title": "t", "what": "w", "causes": [], "fixes": ["f"], "read": "docs/design/x.md", "exit": 1, "matches": ["("]}]
        with open(os.path.join(t, CATALOG), "w") as f:
            yaml.safe_dump(bad, f)
        assert run(t, write=False) == 1, "an entry with no cause and a bad pattern must fail"
    print("check-error-catalog: self-test ok")
    return 0


def run(repo, write):
    problems = []
    try:
        data = load(repo)
    except Exception as e:  # noqa: BLE001
        print(f"check-error-catalog: cannot read {CATALOG}: {e}", file=sys.stderr)
        return 2
    codes = check_entries(repo, data, problems)
    check_citations(repo, codes, problems)
    check_codes_doc(repo, data, codes, problems)
    ref = os.path.join(repo, REFERENCE)
    want = render(data)
    if write:
        os.makedirs(os.path.dirname(ref), exist_ok=True)
        with open(ref, "w") as f:
            f.write(want)
        print(f"wrote {REFERENCE}")
    else:
        have = ""
        if os.path.exists(ref):
            with open(ref) as f:
                have = f.read()
        if have != want:
            problems.append(f"{REFERENCE} is out of date: regenerate with ops/ci/check-error-catalog.py --write")
    pages = os.path.join(repo, PAGES)
    want_pages = {f"{e['code']}.md": render_page(e) for e in data.get("entries") or []
                  if CODE_SHAPE.match(str(e.get("code", "")))}
    have_pages = sorted(n for n in os.listdir(pages) if n.endswith(".md")) if os.path.isdir(pages) else []
    if write:
        os.makedirs(pages, exist_ok=True)
        for name in have_pages:
            if name not in want_pages:
                os.remove(os.path.join(pages, name))
        for name, text in want_pages.items():
            with open(os.path.join(pages, name), "w") as f:
                f.write(text)
        print(f"wrote {PAGES}/ ({len(want_pages)} pages)")
    else:
        stale = []
        for name, text in want_pages.items():
            have_page = ""
            if name in have_pages:
                with open(os.path.join(pages, name)) as f:
                    have_page = f.read()
            if have_page != text:
                stale.append(name)
        extra = [n for n in have_pages if n not in want_pages]
        if stale or extra:
            problems.append(f"{PAGES}/ is out of date ({len(stale)} page(s) differ or are missing, "
                            f"{len(extra)} have no catalog entry): regenerate with ops/ci/check-error-catalog.py --write")
    ftl = os.path.join(repo, FTL)
    want_ftl = render_ftl(data)
    if write:
        os.makedirs(os.path.dirname(ftl), exist_ok=True)
        with open(ftl, "w") as f:
            f.write(want_ftl)
        print(f"wrote {FTL}")
    else:
        have_ftl = ""
        if os.path.exists(ftl):
            with open(ftl) as f:
                have_ftl = f.read()
        if have_ftl != want_ftl:
            problems.append(f"{FTL} is out of date: regenerate with ops/ci/check-error-catalog.py --write, "
                            "then fill the locales with ops/scripts/translate-ftl.py --apply --catalog goat-errors")
    for p in problems:
        print(p)
    if problems:
        print(f"check-error-catalog: {len(problems)} problem(s)")
        return 1
    print(f"check-error-catalog: ok ({len(codes)} codes)")
    return 0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--repo", default=os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "..")))
    ap.add_argument("--write", action="store_true", help=f"regenerate {REFERENCE}, {PAGES}/ and {FTL}")
    ap.add_argument("--self-test", action="store_true")
    a = ap.parse_args()
    if a.self_test:
        return self_test()
    return run(a.repo, a.write)


if __name__ == "__main__":
    sys.exit(main())
