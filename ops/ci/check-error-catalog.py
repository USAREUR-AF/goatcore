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
# The English Fluent catalog of the entries' prose — the arbiter the other
# locales are translated from (docs/design/goat-cli-i18n.md §"CLI catalog
# currency"). Generated from CATALOG, never edited by hand; the translations
# beside it are filled by ops/scripts/translate-ftl.py --catalog goat-errors.
FTL = "crates/goat-errors/i18n/en/errors.ftl"
CODES_DOC = "docs/design/goat-cli-error-codes.md"
GOAT_ERRORS_RS = "crates/goat-cli/src/errors.rs"
RUST_DIRS = ("crates/goatbuilder/src", "crates/goat-cli/src", "crates/goat-errors/src")
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
        for m in e.get("matches") or []:
            try:
                re.compile(m)
            except re.error as err:
                problems.append(f"{where}: pattern {m!r} does not compile: {err}")
    return seen


def check_citations(repo, data, codes, problems):
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
    mapped_goat_codes = set()
    if os.path.exists(p):
        with open(p) as f:
            for n, line in enumerate(f, 1):
                for c in E_CODE_IN_RS.findall(line):
                    mapped_goat_codes.add(c)
                    if c not in codes:
                        problems.append(f"{GOAT_ERRORS_RS}:{n}: {c} is not in {CATALOG}")
    # E_* is goat's emitted namespace. Catalog-only pattern entries remain
    # valid for external tools and goatbuilder, but a goat E_* entry must be
    # backed by an actual GoatError mapping.
    catalogued_goat_codes = {
        str(e.get("code", ""))
        for e in data.get("entries") or []
        if str(e.get("code", "")).startswith("E_") and e.get("tool") == "goat"
    }
    for c in sorted(catalogued_goat_codes - mapped_goat_codes):
        problems.append(
            f"{CATALOG}: {c} is in goat's E_* namespace but has no "
            f"#[error(\"{c}\")] mapping in {GOAT_ERRORS_RS}"
        )


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
            out.append("| `{code}` | {tool} | **{title}** — {what} | {causes} | {fixes} | `{read}` | {exit} |".format(
                code=e["code"], tool=e.get("tool", ""), title=str(e.get("title", "")).replace("|", "\\|"),
                what=str(e.get("what", "")).replace("|", "\\|"), causes=cell(e.get("causes") or []),
                fixes=cell(f"`{x}`" for x in (e.get("fixes") or [])), read=e.get("read", ""),
                exit=e.get("exit", "")))
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
        os.makedirs(os.path.join(t, "crates/goat-cli/src"))
        write(os.path.join(t, "docs/design/x.md"), "x\n")
        good = {"schema": "goat-errors/v1", "entries": [
            {"code": "E_TEST", "tool": "goat", "title": "t", "what": "w", "causes": ["c"], "fixes": ["f"], "read": "docs/design/x.md", "exit": 1},
            {"code": "GB-001", "tool": "goatbuilder", "title": "t", "what": "w", "causes": ["c"], "fixes": ["f"], "read": "docs/design/x.md", "exit": 2, "matches": ["unknown component"]},
            {"code": "X-SSH-001", "tool": "ssh", "title": "t", "what": "w", "causes": ["c"], "fixes": ["f"], "read": "docs/design/x.md", "matches": ["Permission denied \\(publickey"]},
        ]}
        with open(os.path.join(t, CATALOG), "w") as f:
            yaml.safe_dump(good, f)
        write(os.path.join(t, "crates/goatbuilder/src/a.rs"), 'coded("GB-001", x)\n')
        write(os.path.join(t, GOAT_ERRORS_RS), '#[error("E_TEST")]\n')
        write(os.path.join(t, CODES_DOC), '| `E_TEST` | test | 1 | fix |\n')
        write(os.path.join(t, REFERENCE), render(good))
        os.makedirs(os.path.dirname(os.path.join(t, FTL)))
        write(os.path.join(t, FTL), render_ftl(good))
        assert run(t, write=False) == 0, "a complete catalog must pass"
        assert ftl_text("run `x { $y }`") == 'run `x {"{"} $y {"}"}`', "braces are escaped as Fluent literals"
        assert "gb-001-cause-1 = c" in render_ftl(good), "causes are numbered from 1"
        orphan = {"code": "E_ORPHAN", "tool": "goat", "title": "t", "what": "w", "causes": ["c"], "fixes": ["f"], "read": "docs/design/x.md", "exit": 1}
        good["entries"].append(orphan)
        with open(os.path.join(t, CATALOG), "w") as f:
            yaml.safe_dump(good, f)
        write(os.path.join(t, CODES_DOC), '| `E_TEST` | test | 1 | fix |\n| `E_ORPHAN` | orphan | 1 | fix |\n')
        write(os.path.join(t, REFERENCE), render(good))
        write(os.path.join(t, FTL), render_ftl(good))
        assert run(t, write=False) == 1, "a catalogued goat E_* without a GoatError mapping must fail"
        good["entries"].pop()
        with open(os.path.join(t, CATALOG), "w") as f:
            yaml.safe_dump(good, f)
        write(os.path.join(t, CODES_DOC), '| `E_TEST` | test | 1 | fix |\n')
        write(os.path.join(t, REFERENCE), render(good))
        write(os.path.join(t, FTL), render_ftl(good))
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
    check_citations(repo, data, codes, problems)
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
    ap.add_argument("--write", action="store_true", help=f"regenerate {REFERENCE} and {FTL}")
    ap.add_argument("--self-test", action="store_true")
    a = ap.parse_args()
    if a.self_test:
        return self_test()
    return run(a.repo, a.write)


if __name__ == "__main__":
    sys.exit(main())
