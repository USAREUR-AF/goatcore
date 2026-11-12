#!/usr/bin/env python3
"""goatbuilder keeps its promise to the last release (docs/design/goatbuilder-v1-gate.md, group A).

Four surfaces are what other people build on. This script holds a build of
goatbuilder to three of them, against the baseline kept beside it
(baseline.yaml names the release it was recorded from). The fourth, A2's
configuration files, is crates/goatbuilder/tests/config_compat.rs:

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
  compat.py check   --bin <goatbuilder> [--repo DIR] [--only surface,documents,errors]
  compat.py refresh --tag goatbuilder/vX.Y.Z --bin <that release's binary> --sums <its SHA256SUMS> [--repo DIR]

`check` prints one row per kept item and exits 0 when every row holds, 1 when
any does not (each failure names what to restore, or the file that would
excuse it), 2 when an input cannot be read.

`refresh` is the release step (README.md, "When a release is published"): it
records the named release as the new baseline. It reads the command tree and
the documents from the released binary and the error codes from the catalog at
the tag. It refuses a binary whose
SHA-256 is not the goatbuilder binary entry of the SHA256SUMS published with
the tag (`--sums`), or that disagrees with releases.sha256 for a release
already pinned in the upgrade tests' list (PINS); `--version` alone proves nothing.

Needs PyYAML.
"""
from __future__ import annotations

import argparse
import copy
import datetime
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile

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


OPTION_HEAD = re.compile(r"^ {2,6}(?:-[A-Za-z0-9], )?(--[A-Za-z0-9][A-Za-z0-9-]*)")
ARG_HEAD = re.compile(r"^ {2}([<\[][A-Z0-9_]+[>\]])(?:\.\.\.)? ")
VALUES = re.compile(r"\[possible values: ([^\]]+)\]")


def parse_values(help_text: str) -> dict:
    """{"--output": ["table", "json", "yaml"], "<SHELL>": [...]}: each option or
    argument of one `--help` page that lists its possible values, read across
    wrapped lines."""
    entries = []
    for line in help_text.splitlines():
        m = OPTION_HEAD.match(line) or ARG_HEAD.match(line)
        if m:
            name = m.group(1)
            if name.startswith("["):
                name = "<" + name[1:-1] + ">"
            entries.append([name, line[m.end():]])
        elif not line.strip():
            continue  # the long help form leaves blank lines inside an entry
        elif entries and line.startswith("   "):
            entries[-1][1] += " " + line.strip()
        else:
            entries.append([None, ""])  # a section heading or prose ends the entry
    out = {}
    for name, text in entries:
        m = VALUES.search(" ".join(text.split())) if name else None
        if m:
            out[name] = [v.strip() for v in m.group(1).split(",")]
    return out


def surface_values(binary: str, tree: dict) -> dict:
    """Each command's possible values, from its `--help` (English, its own HOME)."""
    home = tempfile.mkdtemp(prefix="compat-home-")
    env = dict(_env(), HOME=home)
    try:
        for cmd in tree:
            r = subprocess.run([binary, *cmd.split()[1:], "--help"], capture_output=True, text=True, env=env)
            tree[cmd]["values"] = parse_values(r.stdout) if r.returncode == 0 else {}
    finally:
        shutil.rmtree(home, ignore_errors=True)
    return tree


def _deprecated(deps: list, command: str, flag: str | None = None, value: str | None = None) -> bool:
    if value is not None:
        return any(d.get("command") == command and d.get("flag") == flag and d.get("value") == value for d in deps)
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
        bvals, nvals = base[cmd].get("values") or {}, new[cmd].get("values") or {}
        for flag in sorted(bvals):
            if flag not in nvals:
                # A flag that is gone is reported above; one still here, or an
                # argument, that no longer lists its values changed meaning.
                if flag in nflags or flag.startswith("<"):
                    out.append(f"{cmd} {flag}: listed its possible values {', '.join(bvals[flag])} in the last release and lists none now")
                continue
            for v in bvals[flag]:
                if v not in nvals[flag] and not _deprecated(deps, cmd, flag, v):
                    out.append(f"{cmd} {flag}: the value `{v}` is no longer accepted; restore it, or name it in deprecations.yaml (with `value: {v}`) and its replacement")
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
        if "value" in d and not (d.get("flag") and str(d.get("value") or "").strip()):
            out.append(f"{where}: `value:` deprecates one value of a flag or argument; it needs the `flag:` (or `<ARG>`) it belongs to")
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


def conditional_problems(cond: list) -> list:
    """What is wrong with conditional.yaml's entries: each names a format, an array
    of entries, a field, the condition, and why the field is carried only then."""
    out = []
    for i, c in enumerate(cond):
        where = f"conditional.yaml entry {i + 1}"
        if not isinstance(c, dict):
            out.append(f"{where}: not a mapping")
            continue
        for key in ("format", "items", "field", "reason"):
            if not (isinstance(c.get(key), str) and c[key].strip()):
                out.append(f"{where}: `{key}:` is missing or empty")
        if isinstance(c.get("items"), str) and not c["items"].endswith("[]"):
            out.append(f"{where}: `items:` names an array of entries, written `name[]`")
        if not (isinstance(c.get("present_when"), dict) and c["present_when"]):
            out.append(f"{where}: `present_when:` names the entries that carry the field, as field: value")
    return out


def _formatted(v, fmt: str, path: str = ""):
    """(path, object) for every object in `v` whose `schema` is `fmt`, the root included."""
    if isinstance(v, dict):
        if v.get("schema") == fmt:
            yield path, v
        for key, val in v.items():
            yield from _formatted(val, fmt, f"{path}.{key}" if path else key)
    elif isinstance(v, list):
        for i, val in enumerate(v):
            yield from _formatted(val, fmt, f"{path}[{i}]")


def _conditional(base, new, cond: list):
    """Copies of both documents without each conditional field, and what the new one
    breaks of the condition: an entry the condition names that lacks the field, or a
    value whose type differs from the release's."""
    base, new = copy.deepcopy(base), copy.deepcopy(new)
    out = []
    for c in cond:
        arr, field, when = c["items"][:-2], c["field"], c["present_when"]
        kinds = set()
        for _, obj in _formatted(base, c["format"]):
            for item in obj.get(arr) or []:
                if isinstance(item, dict) and field in item:
                    kinds.add(_kind(item.pop(field)))
        for path, obj in _formatted(new, c["format"]):
            for i, item in enumerate(obj.get(arr) or []):
                if not isinstance(item, dict):
                    continue
                at = f"{path + '.' if path else ''}{arr}[{i}].{field}"
                named = all(item.get(k) == v for k, v in when.items())
                if field in item:
                    k = _kind(item.pop(field))
                    if kinds and k not in kinds and k != "null":
                        out.append(f"{at}: was {'/'.join(sorted(kinds))}, is now {k} (retyped)")
                elif named:
                    cond_text = ", ".join(f"{k} is {v}" for k, v in when.items())
                    out.append(f"{at}: the field is gone from an entry where {cond_text} (conditional.yaml: {c['format']} carries it there)")
    return base, new, out


def document_problems(base, new, conditional=()) -> list:
    """What the last release's document carries that this build's does not.

    A field `conditional` names (conditional.yaml) is held per entry instead of by
    the merged shape: it may be absent from an entry its condition does not name."""
    out = []
    # A document that carried no version may gain one (an added field); one that
    # carried a version keeps exactly that version.
    if isinstance(base, dict) and isinstance(new, dict) and "schema" in base and base.get("schema") != new.get("schema"):
        out.append(f"schema: the document's version was {base.get('schema')!r}, is now {new.get('schema')!r}")
    base, new, held = _conditional(base, new, list(conditional))
    return out + held + _shape_problems(shape(base), shape(new), "")


def exit_problems(d: dict) -> list:
    """A kept invocation's `exit` is its release's (`release_exit`), or moved with a reason.

    Exit codes are a promised surface: a pull request that changes one edits
    `exit` and says why in `exit_changed`, so the change is reviewed.
    """
    name = d.get("name", "?")
    if "release_exit" not in d:
        return [f"{name}: no `release_exit`; rerun compat.py refresh to record the release's exit code"]
    reason = d.get("exit_changed")
    if d.get("exit") != d["release_exit"] and not (isinstance(reason, str) and reason.strip()):
        return [f"{name}: `exit` is {d.get('exit')} but the release exited {d['release_exit']}; "
                f"give the reason in `exit_changed:`, or restore {d['release_exit']}"]
    return []


def release_values_digest(tag: str, docs: list) -> str:
    """The SHA-256 over what the release did for each kept invocation: its tag,
    and every (name, format, args, release_exit). refresh records it in
    baseline.yaml; an `exit` edit with its reason leaves it unchanged."""
    rows = sorted(
        [d["name"], d["format"], list(d.get("args") or []), d.get("release_exit")]
        for d in docs
    )
    text = json.dumps({"release": tag, "documents": rows}, sort_keys=True, ensure_ascii=False)
    return hashlib.sha256(text.encode()).hexdigest()


def release_values_problems(tag: str, docs: list, meta: dict) -> list:
    """The kept release values are the ones refresh recorded from the published binary."""
    want = meta.get("documents_sha256")
    if not want:
        return ["baseline.yaml has no documents_sha256; rerun compat.py refresh with the published binary and SHA256SUMS"]
    if release_values_digest(tag, docs) != want:
        return [f"documents/manifest.yaml: a kept name, format, args or release_exit differs from what refresh recorded "
                f"from {tag} (documents_sha256 in baseline.yaml); change `exit` with `exit_changed:` instead, "
                f"or rerun compat.py refresh with the published binary"]
    return []


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

    def not_kept(self) -> dict:
        """The verbs with no kept document, each with the reason."""
        return _read_yaml(os.path.join(self.root, "documents", "manifest.yaml")).get("not_kept") or {}

    def document(self, d: dict):
        with open(os.path.join(self.root, "documents", d["name"] + "." + d["format"])) as f:
            return json.load(f) if d["format"] == "json" else yaml.safe_load(f)

    def deprecations(self) -> list:
        return _read_yaml(os.path.join(self.root, "deprecations.yaml")).get("deprecations") or []

    def conditional(self) -> list:
        """The fields a document format carries only on some entries, each with its reason."""
        path = os.path.join(self.root, "conditional.yaml")
        return (_read_yaml(path).get("conditional") or []) if os.path.isfile(path) else []


def _env() -> dict:
    env = dict(os.environ)
    for k in ("GOAT_EXPLAIN", "GOATBUILDER_UPDATE_URL", "GOATBUILDER_TIMEOUT", "CLICOLOR_FORCE", "RUST_LOG"):
        env.pop(k, None)
    env.update({"GOAT_LANG": "en", "NO_COLOR": "1", "LC_ALL": "C.UTF-8"})
    return env


SCRATCH = "{scratch}"


def invocation_verb(args: list, verbs: list):
    """The top-level verb an invocation runs: its first argument that names one."""
    return next((a for a in args if a in verbs), None)


def run_document(binary: str, repo: str, d: dict):
    """Run one kept invocation. A verb that changes state runs on a scratch copy:
    each fixture named in `scratch:` is copied from documents/fixtures/ into a
    fresh directory, `{scratch}` in the arguments names it, and its path is
    written back as `{scratch}` in the output, so the kept document names no
    machine's directory."""
    args = list(d["args"])
    tmp = None
    if d.get("scratch"):
        tmp = tempfile.mkdtemp(prefix="compat-scratch-")
        for f in d["scratch"]:
            shutil.copytree(os.path.join(HERE, "documents", "fixtures", f), os.path.join(tmp, f))
        args = [a.replace(SCRATCH, tmp) for a in args]
    try:
        r = subprocess.run([binary, "-o", d["format"], *args], capture_output=True, text=True, env=_env(), cwd=repo)
    finally:
        if tmp:
            shutil.rmtree(tmp, ignore_errors=True)
    out = r.stdout.replace(tmp, SCRATCH) if tmp else r.stdout
    try:
        doc = json.loads(out) if d["format"] == "json" else yaml.safe_load(out)
    except (ValueError, yaml.YAMLError) as e:
        return r.returncode, None, f"not a {d['format']} document ({e}); stderr: {r.stderr.strip()[:300]}"
    return r.returncode, doc, None


# ---------------------------------------------------------------- check


def check_all(repo: str, root: str, binary: str, only=("surface", "documents", "errors")) -> list:
    """Rows of (ok, surface, item, detail)."""
    b = Baseline(root)
    rows = []
    pinned = load_pins(repo).get((b.release, b.meta.get("binary")))
    if pinned and pinned == b.meta.get("binary_sha256"):
        rows.append((True, "baseline", b.release, f"recorded from the published {b.meta['binary']} ({pinned[:16]}…), pinned in {PINS}"))
    elif not pinned:
        rows.append((False, "baseline", b.release, f"{PINS} pins no {b.meta.get('binary')} of {b.release}; the upgrade tests own that file: add the release's lines there from its SHA256SUMS"))
    else:
        rows.append((False, "baseline", b.release, f"baseline.yaml's binary_sha256 is not the {PINS} pin for {b.meta.get('binary')} of {b.release}; rerun compat.py refresh with the published binary and SHA256SUMS"))
    for p in release_values_problems(b.release, b.documents(), b.meta):
        rows.append((False, "baseline", "documents/manifest.yaml", p))
    deps = b.deprecations()
    for p in deprecation_problems(deps):
        rows.append((False, "surface", "deprecations.yaml", p))
    cond = b.conditional()
    for p in conditional_problems(cond):
        rows.append((False, "documents", "conditional.yaml", p))

    if "surface" in only:
        r = subprocess.run([binary, "docs", "--markdown"], capture_output=True, text=True, env=_env(), cwd=repo)
        if r.returncode != 0:
            rows.append((False, "surface", "docs --markdown", f"exit {r.returncode}: {r.stderr.strip()[:300]}"))
        else:
            new = surface_values(binary, parse_reference(r.stdout))
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
            p = exit_problems(d) + document_problems(b.document(d), doc, cond)
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


def _git(repo: str, *args: str) -> str:
    return subprocess.run(["git", "-C", repo, *args], capture_output=True, text=True, check=True).stdout


# The one list of published release checksums, shared with the upgrade tests
# (gate E1 to E3), which own it: `<sha256>  vX.Y.Z/goatbuilder-<os>-<arch>`, as
# SHA256SUMS writes a line. compat reads it and never writes it; adding a
# release to it is a deliberate step in the pull request that names the release
# in the upgrade tests.
PINS = "crates/goatbuilder/tests/upgrade/releases.sha256"
RELEASE_BINARY = re.compile(r"^goatbuilder-(linux|darwin)-(amd64|arm64)$")


def _sha256(path: str) -> str:
    with open(path, "rb") as f:
        return hashlib.sha256(f.read()).hexdigest()


def load_pins(repo: str) -> dict:
    """{(tag, file): sha256} from the shared pin list, e.g. ("goatbuilder/v0.5.1", "goatbuilder-linux-amd64")."""
    pins = {}
    path = os.path.join(repo, PINS)
    if os.path.exists(path):
        with open(path) as f:
            for line in f:
                if line.startswith("#") or "  " not in line:
                    continue
                sha, rel = line.rstrip("\n").split("  ", 1)
                version, _, asset = rel.partition("/")
                if version and asset:
                    pins[(f"goatbuilder/{version}", asset)] = sha
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


def refresh(repo: str, root: str, tag: str, binary: str, sums=None) -> int:
    if not sums:
        print(f"compat: refresh needs --sums, the SHA256SUMS published with {tag}", file=sys.stderr)
        return 2
    problem = release_binary_problem(binary, sums, tag, load_pins(repo))
    if problem:
        print(f"compat: {problem}", file=sys.stderr)
        return 2
    name = [n for n, h in _sums_entries(sums).items() if h == _sha256(binary) and RELEASE_BINARY.match(n)][0]
    commit = _git(repo, "rev-parse", f"{tag}^{{commit}}").strip()
    if (tag, name) not in load_pins(repo):
        print(f"compat: {PINS} pins no {name} of {tag} yet; `check` refuses the baseline until the upgrade tests' owners add the release's lines from its SHA256SUMS", file=sys.stderr)

    b = Baseline(root)
    _refresh_last_release(repo, root, tag, binary, b)

    meta = dict(b.meta)
    meta.update({
        "release": tag,
        "commit": commit,
        "binary": name,
        "binary_sha256": _sha256(binary),
        "sums_sha256": _sha256(sums),
        "documents_sha256": release_values_digest(tag, Baseline(root).documents()),
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
        json.dump(surface_values(binary, parse_reference(md)), f, indent=1, sort_keys=True)
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
        d["release_exit"] = rc
        d.pop("exit_changed", None)  # the release's own exit now; the reason was for the change before it
        with open(os.path.join(root, "documents", d["name"] + "." + d["format"]), "w") as f:
            if d["format"] == "json":
                json.dump(doc, f, indent=2, ensure_ascii=False)
                f.write("\n")
            else:
                yaml.safe_dump(doc, f, sort_keys=False, allow_unicode=True)
    _write_manifest(os.path.join(root, "documents", "manifest.yaml"), None, {"documents": docs, "not_kept": b.not_kept()})


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
    c.add_argument("--only", default="surface,documents,errors")
    r = sub.add_parser("refresh")
    r.add_argument("--tag", required=True)
    r.add_argument("--bin", required=True)
    r.add_argument("--repo", default=os.path.abspath(os.path.join(HERE, "..", "..", "..")))
    r.add_argument("--sums", required=True, help="the SHA256SUMS published with the tag; the binary's sha256 must be its entry")
    a = ap.parse_args(argv)
    binary = os.path.abspath(a.bin)
    if not os.access(binary, os.X_OK):
        print(f"compat: {a.bin} is not an executable goatbuilder", file=sys.stderr)
        return 2
    if a.cmd == "refresh":
        return refresh(a.repo, HERE, a.tag, binary, sums=a.sums)
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
