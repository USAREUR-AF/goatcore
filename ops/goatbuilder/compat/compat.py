#!/usr/bin/env python3
"""goatbuilder keeps its promise to the last release (docs/design/goatbuilder-v1-gate.md, group A).

Four surfaces are what other people build on. This script holds a build of
goatbuilder to each, against the baseline kept beside it (baseline.yaml names
the release it was recorded from):

  configs    A2  every kept configuration file, from each release since the
                 first, validates, or is refused the way its release refused it
  surface    A3  every command and flag of the last release is still there
                 (and still takes a value if it took one, and no optional
                 argument became required), unless deprecations.yaml names it
  documents  A4  every kept `-o json` / `-o yaml` document of the last release
                 is still produced by the same invocation as a superset: the
                 same version, no field removed, no field retyped, no list
                 that held entries now empty
  errors     A5  every GB-nnn code of the last release is in ops/errors/catalog.yaml
                 with the same title, unless the entry carries `retired:`

usage:
  compat.py check   --bin <goatbuilder> [--repo DIR] [--only configs,surface,documents,errors]
  compat.py refresh --tag goatbuilder/vX.Y.Z --bin <that release's binary> --sums <its SHA256SUMS> [--repo DIR] [--only configs]

`check` prints one row per kept item and exits 0 when every row holds, 1 when
any does not (each failure names what to restore, or the file that would
excuse it), 2 when an input cannot be read.

`refresh` is the release step (README.md, "When a release is published"): it
records the named release as the new baseline. It reads the command tree and
the documents from the released binary, the error codes from the catalog at
the tag, and adds every configuration file of the tag's tree, and every
example in its documents, that is not already kept. It refuses a binary whose
SHA-256 is not the goatbuilder binary entry of the SHA256SUMS published with
the tag (`--sums`), or that disagrees with releases.sha256 for a release
already pinned there; `--version` alone proves nothing.

Needs PyYAML.
"""
from __future__ import annotations

import argparse
import datetime
import hashlib
import json
import os
import re
import subprocess
import sys

try:
    import yaml
except ImportError:  # pragma: no cover - the CI step installs it
    print("compat: PyYAML missing (pip install pyyaml)", file=sys.stderr)
    sys.exit(2)

HERE = os.path.dirname(os.path.abspath(__file__))
CATALOG = "ops/errors/catalog.yaml"

# ---------------------------------------------------------------- A3: the command surface

HEADING = re.compile(r"^## `(goatbuilder(?: [^`]+)?)`\s*$")
FLAG_CELL = re.compile(r"^\|\s*`([^`]+)`\s*\|")
FLAG_NAME = re.compile(r"^(--?[A-Za-z0-9][A-Za-z0-9-]*)(?:\s+<[^>]+>)?$")
ARG_CELL = re.compile(r"^\|\s*`(<[^>]+>|\[[^\]]+\])(\.\.\.)?`\s*\|")


def parse_reference(text: str) -> dict:
    """The command tree from `goatbuilder docs --markdown`.

    Returns {"goatbuilder deploy": {"flags": {"--env": True, "-e": True}, "args": [...]}}:
    each flag (long name and short alias alike) maps to whether it takes a value;
    each positional argument is {"name", "required"}, in order.
    """
    tree: dict = {}
    cmd = None
    section = None
    for line in text.splitlines():
        m = HEADING.match(line)
        if m:
            cmd = m.group(1)
            tree[cmd] = {"flags": {}, "args": []}
            section = None
            continue
        if cmd is None:
            continue
        if line.startswith("**"):
            section = line.strip("* ").lower()
            continue
        if section == "options":
            m = FLAG_CELL.match(line)
            if not m or m.group(1) == "Flag":
                continue
            names = [n.strip() for n in m.group(1).split(",")]
            takes_value = "<" in m.group(1)
            for n in names:
                fm = FLAG_NAME.match(n)
                if fm:
                    tree[cmd]["flags"][fm.group(1)] = takes_value
        elif section == "arguments":
            m = ARG_CELL.match(line)
            if not m:
                continue
            raw = m.group(1)
            tree[cmd]["args"].append({"name": raw[1:-1], "required": raw.startswith("<")})
    return tree


def _deprecated(deps: list, command: str, flag: str | None = None) -> bool:
    for d in deps:
        if d.get("command") == command and d.get("flag") == flag:
            return True
        # A whole command deprecated excuses its flags too.
        if d.get("command") == command and not d.get("flag") and flag is not None:
            return True
    return False


def surface_problems(base: dict, new: dict, deps: list) -> list:
    """What the last release's command tree has that this build's does not."""
    out = []
    for cmd in sorted(base):
        if cmd not in new:
            # A removed parent's subcommands are covered by the parent's row.
            parent = cmd.rsplit(" ", 1)[0]
            if " " in cmd and parent in base and parent not in new:
                continue
            if not _deprecated(deps, cmd):
                out.append(f"{cmd}: the command is gone; restore it, or name it in deprecations.yaml with its replacement")
            continue
        bflags, nflags = base[cmd]["flags"], new[cmd]["flags"]
        for flag in sorted(bflags):
            if flag not in nflags:
                if not _deprecated(deps, cmd, flag):
                    out.append(f"{cmd} {flag}: the flag is gone or renamed; restore it, or name it in deprecations.yaml with its replacement")
            elif bflags[flag] != nflags[flag]:
                was = "took a value" if bflags[flag] else "took no value"
                out.append(f"{cmd} {flag}: the flag {was} in the last release and changes meaning here")
        bargs, nargs = base[cmd]["args"], new[cmd]["args"]
        if len(nargs) < len(bargs) and not _deprecated(deps, cmd):
            out.append(f"{cmd}: had {len(bargs)} positional argument(s), now {len(nargs)}")
        for i, (b, n) in enumerate(zip(bargs, nargs)):
            if not b["required"] and n["required"]:
                out.append(f"{cmd}: positional argument {i + 1} ({b['name']}) was optional and is now required")
    return out


def deprecation_problems(deps: list) -> list:
    """Every deprecation entry names what it deprecates, its replacement and the release."""
    out = []
    for i, d in enumerate(deps):
        where = f"deprecations.yaml entry {i + 1}"
        if not str(d.get("command", "")).startswith("goatbuilder"):
            out.append(f"{where}: `command:` must name a goatbuilder command")
        if not d.get("replacement"):
            out.append(f"{where}: `replacement:` must name what to use instead (the warning prints it)")
        if not re.match(r"^goatbuilder/v\d+\.\d+\.\d+$", str(d.get("deprecated_in", ""))):
            out.append(f"{where}: `deprecated_in:` must name the release, goatbuilder/vX.Y.Z")
    return out


# ---------------------------------------------------------------- A4: the documents


def _kind(v) -> str:
    if v is None:
        return "null"
    if isinstance(v, bool):
        return "boolean"
    if isinstance(v, (int, float)):
        return "number"
    if isinstance(v, str):
        return "string"
    if isinstance(v, list):
        return "array"
    if isinstance(v, dict):
        return "object"
    return type(v).__name__


def shape(v):
    """The structure of a document with its values left out: types, fields, list items merged."""
    k = _kind(v)
    if k == "object":
        return {"type": "object", "fields": {key: shape(val) for key, val in v.items()}}
    if k == "array":
        items = None
        for item in v:
            items = merge(items, shape(item))
        return {"type": "array", "items": items}
    return {"type": k}


def merge(a, b):
    if a is None:
        return b
    if b is None:
        return a
    if a["type"] == "null":
        return b
    if b["type"] == "null":
        return a
    if a["type"] != b["type"]:
        types = set(a.get("of", [a["type"]])) | set(b.get("of", [b["type"]]))
        return {"type": "mixed", "of": sorted(types)}
    if a["type"] == "object":
        fields = dict(a["fields"])
        for key, val in b["fields"].items():
            fields[key] = merge(fields.get(key), val)
        return {"type": "object", "fields": fields}
    if a["type"] == "array":
        return {"type": "array", "items": merge(a["items"], b["items"])}
    return a


def _shape_problems(base, new, path: str) -> list:
    if base is None or base["type"] == "null" or new["type"] == "null":
        return []  # nothing was promised about a value that was empty
    if base["type"] == "mixed":
        missing = set(base["of"]) - set(new.get("of", [new["type"]]))
        return [f"{path}: was {'/'.join(base['of'])}, is now {new['type']}"] if missing else []
    if base["type"] != new["type"]:
        return [f"{path}: was {base['type']}, is now {new['type']} (retyped)"]
    out = []
    if base["type"] == "object":
        for key, val in base["fields"].items():
            p = f"{path}.{key}" if path else key
            if key not in new["fields"]:
                out.append(f"{p}: the field is gone")
            else:
                out += _shape_problems(val, new["fields"][key], p)
    elif base["type"] == "array" and base["items"] is not None:
        if new["items"] is None:
            out.append(f"{path}: held entries in the last release and is now empty, so its fields cannot be shown to remain")
        else:
            out += _shape_problems(base["items"], new["items"], path + "[]")
    return out


def document_problems(base, new) -> list:
    """What the last release's document carries that this build's does not."""
    out = []
    # A document that carried no version may gain one (an added field); one that
    # carried a version keeps exactly that version.
    if isinstance(base, dict) and isinstance(new, dict) and "schema" in base and base.get("schema") != new.get("schema"):
        out.append(f"schema: the document's version was {base.get('schema')!r}, is now {new.get('schema')!r}")
    return out + _shape_problems(shape(base), shape(new), "")


# ---------------------------------------------------------------- A5: the error codes


def load_catalog(repo: str) -> list:
    with open(os.path.join(repo, CATALOG)) as f:
        return yaml.safe_load(f).get("entries") or []


def release_codes(entries: list) -> list:
    """The codes the promise covers: goatbuilder's own, GB-nnn."""
    return [{"code": e["code"], "title": e["title"]} for e in entries if str(e.get("code", "")).startswith("GB-")]


def error_problems(base: list, catalog: list) -> list:
    now = {e.get("code"): e for e in catalog}
    out = []
    for b in base:
        e = now.get(b["code"])
        if e is None:
            out.append(f"{b['code']}: in the last release ({b['title']!r}) and gone from {CATALOG}; a code stays in the catalog, marked `retired:` when it is no longer printed")
        elif e.get("title") != b["title"] and not e.get("retired"):
            out.append(f"{b['code']}: the title was {b['title']!r} and is now {e.get('title')!r}; a code is never reused for a different fault: restore the title, or mark the entry `retired:` and give the new fault a new code")
    return out


# ---------------------------------------------------------------- A2: the configuration files


def config_problems(kept: list, outcome) -> list:
    """`outcome(file)` -> (valid|refused|error, printed code or None) for a kept file."""
    out = []
    for k in kept:
        got, code = outcome(k["file"])
        want, want_code = k["expect"], k.get("code")
        # A refusal may come to print the catalog code its release's documents
        # named for it (`named:`); that is the promise kept, not broken.
        if got != want or (want != "valid" and code not in (want_code, k.get("named"))):
            def say(o, c):
                return o + (f" ({c})" if c else "")
            out.append(f"configs/{k['file']} (from {k['release']}): {say(want, want_code)} in its release, {say(got, code)} now")
    return out


REFUSAL_CODE = re.compile(r"^(GB-\d{3}): ")


def refusal_code(doc: dict):
    """The catalog code a refused checks document names: its first failing row's `GB-nnn: ` prefix."""
    for c in doc.get("checks") or []:
        if not c.get("ok"):
            m = REFUSAL_CODE.match(str(c.get("detail", "")))
            return m.group(1) if m else None
    return None


def validate_outcome(binary: str, repo: str, path: str):
    """Run `validate` on one file: valid (exit 0), refused (exit 1, a checks document), or an error and its code."""
    r = subprocess.run([binary, "-o", "json", "validate", "-C", repo, path], capture_output=True, text=True, env=_env(), cwd=repo)
    try:
        doc = json.loads(r.stdout)
    except ValueError:
        doc = {}
    if r.returncode == 0:
        return "valid", None
    if r.returncode == 1 and doc.get("schema") == "goatbuilder.output/v1":
        return "refused", refusal_code(doc)
    return "error", doc.get("code")


# ---------------------------------------------------------------- the baseline on disk


def _read_yaml(path: str):
    with open(path) as f:
        return yaml.safe_load(f)


class Baseline:
    """The files beside this script: what the last release did."""

    def __init__(self, root: str = HERE):
        self.root = root
        meta = _read_yaml(os.path.join(root, "baseline.yaml"))
        self.release = meta["release"]
        self.meta = meta

    def surface(self) -> dict:
        with open(os.path.join(self.root, "surface.json")) as f:
            return json.load(f)

    def errors(self) -> list:
        return _read_yaml(os.path.join(self.root, "errors.yaml"))["codes"]

    def documents(self) -> list:
        return _read_yaml(os.path.join(self.root, "documents", "manifest.yaml"))["documents"]

    def document(self, d: dict):
        with open(os.path.join(self.root, "documents", d["name"] + "." + d["format"])) as f:
            return json.load(f) if d["format"] == "json" else yaml.safe_load(f)

    def configs(self) -> list:
        return _read_yaml(os.path.join(self.root, "configs", "manifest.yaml"))["configs"]

    def deprecations(self) -> list:
        return _read_yaml(os.path.join(self.root, "deprecations.yaml")).get("deprecations") or []


def _env() -> dict:
    env = dict(os.environ)
    for k in ("GOAT_EXPLAIN", "GOATBUILDER_UPDATE_URL", "GOATBUILDER_TIMEOUT", "CLICOLOR_FORCE", "RUST_LOG"):
        env.pop(k, None)
    env.update({"GOAT_LANG": "en", "NO_COLOR": "1", "LC_ALL": "C.UTF-8"})
    return env


def run_document(binary: str, repo: str, d: dict):
    r = subprocess.run([binary, "-o", d["format"], *d["args"]], capture_output=True, text=True, env=_env(), cwd=repo)
    try:
        doc = json.loads(r.stdout) if d["format"] == "json" else yaml.safe_load(r.stdout)
    except (ValueError, yaml.YAMLError) as e:
        return r.returncode, None, f"not a {d['format']} document ({e}); stderr: {r.stderr.strip()[:300]}"
    return r.returncode, doc, None


# ---------------------------------------------------------------- check


def check_all(repo: str, root: str, binary: str, only=("configs", "surface", "documents", "errors")) -> list:
    """Rows of (ok, surface, item, detail)."""
    b = Baseline(root)
    rows = []
    pinned = load_pins(root).get((b.release, b.meta.get("binary")))
    if pinned and pinned == b.meta.get("binary_sha256"):
        rows.append((True, "baseline", b.release, f"recorded from the published {b.meta['binary']} ({pinned[:16]}…)"))
    else:
        rows.append((False, "baseline", b.release, f"baseline.yaml names a binary that {PINS} does not pin; rerun compat.py refresh with the release's binary and SHA256SUMS"))
    deps = b.deprecations()
    for p in deprecation_problems(deps):
        rows.append((False, "surface", "deprecations.yaml", p))

    if "configs" in only:
        for k in b.configs():
            p = config_problems([k], lambda f: validate_outcome(binary, repo, os.path.join(root, "configs", f)))
            rows.append((not p, "configs", k["file"], p[0] if p else k["expect"]))

    if "surface" in only:
        r = subprocess.run([binary, "docs", "--markdown"], capture_output=True, text=True, env=_env(), cwd=repo)
        if r.returncode != 0:
            rows.append((False, "surface", "docs --markdown", f"exit {r.returncode}: {r.stderr.strip()[:300]}"))
        else:
            new = parse_reference(r.stdout)
            problems = surface_problems(b.surface(), new, deps)
            for p in problems:
                rows.append((False, "surface", p.split(":", 1)[0], p.split(":", 1)[1].strip()))
            if not problems:
                rows.append((True, "surface", f"{len(b.surface())} commands", f"every command and flag of {b.release} is here"))

    if "documents" in only:
        for d in b.documents():
            rc, doc, err = run_document(binary, repo, d)
            item = f"{d['name']}.{d['format']}"
            if err:
                rows.append((False, "documents", item, err))
                continue
            p = document_problems(b.document(d), doc)
            if rc != d["exit"]:
                p.append(f"exit {d['exit']} in {b.release}, {rc} now")
            rows.append((not p, "documents", item, "; ".join(p) if p else "a superset of the release's document"))

    if "errors" in only:
        problems = error_problems(b.errors(), load_catalog(repo))
        for p in problems:
            rows.append((False, "errors", p.split(":", 1)[0], p.split(":", 1)[1].strip()))
        if not problems:
            rows.append((True, "errors", f"{len(b.errors())} codes", f"every GB code of {b.release} keeps its title"))
    return rows


# ---------------------------------------------------------------- refresh (the release step)

SCHEMA_LINE = re.compile(r'^schema:\s*"?goatbuilder/v1"?\s*$', re.M)
CONFIG_TREES = ("ops/goatbuilder/fixtures/", "crates/goatbuilder/tests/fixtures/configs/")
FENCE = re.compile(r"^```ya?ml\s*\n(.*?)^```", re.M | re.S)


def _git(repo: str, *args: str) -> str:
    return subprocess.run(["git", "-C", repo, *args], capture_output=True, text=True, check=True).stdout


def tag_configs(repo: str, tag: str) -> list:
    """(source, text) for every configuration file in the tag's tree and every whole example in its documents."""
    out = []
    for path in _git(repo, "ls-tree", "-r", "--name-only", tag).splitlines():
        if path.endswith(".yaml") and path.startswith(CONFIG_TREES):
            text = _git(repo, "show", f"{tag}:{path}")
            if path.startswith(CONFIG_TREES[1]) or SCHEMA_LINE.search(text):
                out.append((path, text))
        elif path.endswith(".md") and path.startswith("docs/"):
            text = _git(repo, "show", f"{tag}:{path}")
            for i, m in enumerate(FENCE.finditer(text)):
                if SCHEMA_LINE.search(m.group(1)):
                    out.append((f"{path}#example-{i + 1}", m.group(1)))
    return out


def _slug(source: str) -> str:
    s = source.replace("#", "-").replace("/", "__")
    return s if s.endswith(".yaml") else s + ".yaml"


PINS = "releases.sha256"
RELEASE_BINARY = re.compile(r"^goatbuilder-(linux|darwin)-(amd64|arm64)$")


def _sha256(path: str) -> str:
    with open(path, "rb") as f:
        return hashlib.sha256(f.read()).hexdigest()


def load_pins(root: str) -> dict:
    """{(tag, file): sha256} from releases.sha256: one `<sha256>  <tag> <file>` per line."""
    pins = {}
    path = os.path.join(root, PINS)
    if os.path.exists(path):
        with open(path) as f:
            for line in f:
                parts = line.split()
                if len(parts) == 3 and not line.startswith("#"):
                    pins[(parts[1], parts[2])] = parts[0]
    return pins


def _sums_entries(sums: str) -> dict:
    """{file: sha256} from a release's SHA256SUMS."""
    out = {}
    with open(sums) as f:
        for line in f:
            parts = line.split()
            if len(parts) == 2:
                out[parts[1].lstrip("*")] = parts[0].lower()
    return out


def release_binary_problem(binary: str, sums: str, tag: str, pins=None):
    """Why `binary` is not the published binary of `tag`, or None when it is.

    The binary's SHA-256 must be the entry the release's SHA256SUMS gives a
    goatbuilder binary, it must report the tag's version, and a release already
    pinned in releases.sha256 must agree with the pin. `--version` alone proves
    nothing: a local build of main reports the same version.
    """
    digest = _sha256(binary)
    names = [n for n, h in _sums_entries(sums).items() if h == digest]
    named = [n for n in names if RELEASE_BINARY.match(n)]
    if not named:
        said = f" (it names {', '.join(names)} with that hash)" if names else ""
        return f"{binary} (sha256 {digest[:16]}…) is not a goatbuilder binary {sums} lists{said}; refresh records a published binary only"
    name = named[0]
    pinned = (pins or {}).get((tag, name))
    if pinned and pinned != digest:
        return f"{sums} gives {name} of {tag} the sha256 {digest[:16]}…, but {PINS} pins {pinned[:16]}…"
    r = subprocess.run([binary, "--version"], capture_output=True, text=True)
    if r.stdout.split()[-1:] != [tag.split("/", 1)[1].lstrip("v")]:
        return f"{binary} reports {r.stdout.strip()!r}, not {tag}"
    return None


def refresh(repo: str, root: str, tag: str, binary: str, only=("configs", "surface", "documents", "errors"), sums=None) -> int:
    if not sums:
        print(f"compat: refresh needs --sums, the SHA256SUMS published with {tag}", file=sys.stderr)
        return 2
    problem = release_binary_problem(binary, sums, tag, load_pins(root))
    if problem:
        print(f"compat: {problem}", file=sys.stderr)
        return 2
    name = [n for n, h in _sums_entries(sums).items() if h == _sha256(binary) and RELEASE_BINARY.match(n)][0]
    commit = _git(repo, "rev-parse", f"{tag}^{{commit}}").strip()
    version = tag.split("/", 1)[1]
    _pin(root, tag, name, _sha256(binary))

    b = Baseline(root)
    if only != ("configs",):
        _refresh_last_release(repo, root, tag, binary, b)
    else:
        print(f"compat: keeping the configuration files of {tag} only; the baseline stays {b.release}")

    # A2: every configuration file of the tag not already kept, with its outcome under the released binary.
    kept = b.configs()
    have = {hashlib.sha256(open(os.path.join(root, "configs", k["file"]), "rb").read()).hexdigest() for k in kept}
    for source, text in tag_configs(repo, tag):
        digest = hashlib.sha256(text.encode()).hexdigest()
        if digest in have:
            continue
        have.add(digest)
        rel = f"{version}/{_slug(source)}"
        dest = os.path.join(root, "configs", rel)
        os.makedirs(os.path.dirname(dest), exist_ok=True)
        with open(dest, "w") as f:
            f.write(text)
        got, code = validate_outcome(binary, repo, dest)
        entry = {"file": rel, "release": tag, "source": source, "expect": got}
        if got != "valid":
            entry["code"] = code
        kept.append(entry)
        print(f"compat: kept configs/{rel}: {got}")
    _write_manifest(os.path.join(root, "configs", "manifest.yaml"), "configs", kept)
    if only == ("configs",):
        return 0

    meta = dict(b.meta)
    meta.update({
        "release": tag,
        "commit": commit,
        "binary": name,
        "binary_sha256": _sha256(binary),
        "sums_sha256": _sha256(sums),
        "recorded": datetime.date.today().isoformat(),
    })
    _write_manifest(os.path.join(root, "baseline.yaml"), None, meta)
    print(f"compat: the baseline is now {tag} ({commit[:12]})")
    return 0


def _refresh_last_release(repo: str, root: str, tag: str, binary: str, b: "Baseline") -> None:
    """Record the surface, the error codes and the documents of the release as the new baseline."""
    # A3: the released binary's command tree.
    md = subprocess.run([binary, "docs", "--markdown"], capture_output=True, text=True, check=True, env=_env()).stdout
    with open(os.path.join(root, "surface.json"), "w") as f:
        json.dump(parse_reference(md), f, indent=1, sort_keys=True)
        f.write("\n")

    # A5: the catalog the tag carried.
    entries = yaml.safe_load(_git(repo, "show", f"{tag}:{CATALOG}"))["entries"]
    with open(os.path.join(root, "errors.yaml"), "w") as f:
        f.write(f"# The GB codes of {tag} and their titles, read from {CATALOG} at the tag by compat.py refresh.\n")
        yaml.safe_dump({"codes": release_codes(entries)}, f, sort_keys=False, allow_unicode=True, width=1000)

    # A4: the released binary's documents for the kept invocations.
    docs = b.documents()
    for d in docs:
        rc, doc, err = run_document(binary, repo, d)
        if err:
            raise SystemExit(f"compat: {d['name']}: {err}")
        d["exit"] = rc
        with open(os.path.join(root, "documents", d["name"] + "." + d["format"]), "w") as f:
            if d["format"] == "json":
                json.dump(doc, f, indent=2, ensure_ascii=False)
                f.write("\n")
            else:
                yaml.safe_dump(doc, f, sort_keys=False, allow_unicode=True)
    _write_manifest(os.path.join(root, "documents", "manifest.yaml"), "documents", docs)


def _pin(root: str, tag: str, name: str, digest: str) -> None:
    """Record a published binary's sha256 in releases.sha256 (once)."""
    if (tag, name) in load_pins(root):
        return
    path = os.path.join(root, PINS)
    new = not os.path.exists(path)
    with open(path, "a") as f:
        if new:
            f.write("# The published binaries the baseline was recorded from: <sha256>  <tag> <file>,\n"
                    "# as each release's SHA256SUMS lists them. Written by compat.py refresh.\n")
        f.write(f"{digest}  {tag} {name}\n")


def _write_manifest(path: str, key, value) -> None:
    header = []
    if os.path.exists(path):
        with open(path) as f:
            for line in f:
                if not line.startswith("#"):
                    break
                header.append(line)
    with open(path, "w") as f:
        f.writelines(header)
        yaml.safe_dump(value if key is None else {key: value}, f, sort_keys=False, allow_unicode=True, width=1000)


# ---------------------------------------------------------------- main


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    c = sub.add_parser("check")
    c.add_argument("--bin", required=True)
    c.add_argument("--repo", default=os.path.abspath(os.path.join(HERE, "..", "..", "..")))
    c.add_argument("--only", default="configs,surface,documents,errors")
    r = sub.add_parser("refresh")
    r.add_argument("--tag", required=True)
    r.add_argument("--bin", required=True)
    r.add_argument("--repo", default=os.path.abspath(os.path.join(HERE, "..", "..", "..")))
    r.add_argument("--sums", required=True, help="the SHA256SUMS published with the tag; the binary's sha256 must be its entry")
    r.add_argument("--only", default="configs,surface,documents,errors",
                   help="`configs` keeps an older release's configuration files without moving the baseline")
    a = ap.parse_args(argv)
    binary = os.path.abspath(a.bin)
    if not os.access(binary, os.X_OK):
        print(f"compat: {a.bin} is not an executable goatbuilder", file=sys.stderr)
        return 2
    if a.cmd == "refresh":
        return refresh(a.repo, HERE, a.tag, binary, tuple(a.only.split(",")), sums=a.sums)
    try:
        rows = check_all(a.repo, HERE, binary, tuple(a.only.split(",")))
    except (OSError, KeyError, yaml.YAMLError) as e:
        print(f"compat: cannot read the baseline: {e}", file=sys.stderr)
        return 2
    release = Baseline(HERE).release
    for ok, surf, item, detail in rows:
        print(f"{'ok  ' if ok else 'FAIL'}  {surf:<9}  {item}  {detail}")
    failed = sum(1 for r in rows if not r[0])
    print(f"compat: {len(rows) - failed} of {len(rows)} hold against {release}" + ("" if not failed else f"; {failed} broken"))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
