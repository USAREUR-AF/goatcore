#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""upstream.py: one upgrade path for every load-bearing upstream dependency.

Each dependency has one manifest, ops/upstream/deps/<name>.yaml (ADR 1262,
docs/design/upstream-upgrade-process.md). The manifest holds the version of
record, the facts that move with it (image digests, checksums, toolchain), the
rules that say where the version appears in this tree, and the patches carried
on top of upstream. Every subcommand reads only the manifest:

  upstream.py list                         dependencies, versions, patch counts
  upstream.py check [NAME ...]             every place agrees with the manifest
  upstream.py bump NAME VERSION [--set fact=value ...] [--dry-run]
                                           rewrite every place to VERSION
  upstream.py patches NAME --src DIR       which carried patches still apply to
                                           an upstream checkout, which are in it
  upstream.py watch [NAME ...] [--json]    newer upstream releases, keyword hits
                                           in their notes, carried patches whose
                                           upstream pull request has shipped
  upstream.py self-test                    each rule kind fails a broken fixture

A manifest named <name>.private.yaml beside <name>.yaml is merged into it when
present. Only an assembled tree carries one; it adds rules and patches that
must not be described in a public repository. Its top-level `exclude` (a list
of path globs) removes those paths from every rule: places that exist only in
the private tree and keep a version on purpose (deploy records, frozen
snapshots, values the release ladder moves) without naming them publicly.

Rule kinds (key `kind`):
  regex   like text, across every file matching `globs` (zero matches is fine).
  text    one file (`path`) or several (`paths`), one regex. Named groups: `version` (or `v`) must equal the
          version of record; any other named group must equal the fact of the
          same name. `min` (default 1) is how many matches the file must hold.
  cargo   every Cargo.toml: a dependency whose name matches `crate` must be an
          exact `=VERSION` pin; every Cargo.lock: such a package must be VERSION.
  pip     every requirements*.txt, pyproject.toml, *.sh, *.yml, *.yaml, *.j2,
          *.py, Dockerfile*: `PACKAGE==X` must have X == VERSION.
  npm     every package.json: "PACKAGE": "X" must be exactly VERSION.
  gomod   every go.mod: `MODULE vX` must be vVERSION (`replace` lines included).
  forbid  every file matching `glob`: `pattern` must not occur (a retired
          image, a tag without a digest).

Repo-wide kinds (cargo, pip, npm, gomod, regex, forbid) take `exclude` and
`include` path globs to scope them.

A line carrying `upstream-pin: allow` (or the older `zenoh-pin-lint: allow`)
is skipped by every rule.
"""

from __future__ import annotations

import argparse
import fnmatch
import json
import os
import re
import subprocess
import sys
import tempfile
import urllib.error
import urllib.request
from dataclasses import dataclass, field
from pathlib import Path

try:
    import yaml
except ImportError:  # pragma: no cover
    sys.exit("upstream.py needs PyYAML (pip install pyyaml)")

ALLOW_MARKERS = ("upstream-pin: allow", "zenoh-pin-lint: allow")
SKIP_DIRS = {".git", "node_modules", "target", ".venv", "__pycache__", ".claude"}
DEPS_DIR = Path("ops/upstream/deps")


# --------------------------------------------------------------------------
# manifest

def load_manifest(root: Path, name: str) -> dict:
    path = root / DEPS_DIR / f"{name}.yaml"
    if not path.exists():
        raise SystemExit(f"no manifest {path}")
    m = yaml.safe_load(path.read_text()) or {}
    priv = root / DEPS_DIR / f"{name}.private.yaml"
    if priv.exists():
        p = yaml.safe_load(priv.read_text()) or {}
        m.setdefault("rules", []).extend(p.get("rules", []))
        m.setdefault("patches", []).extend(p.get("patches", []))
        m.setdefault("facts", {}).update(p.get("facts", {}))
        apply_exclude(m, p.get("exclude") or [])
    m["_path"] = str(path.relative_to(root))
    m.setdefault("facts", {})
    m.setdefault("rules", [])
    m.setdefault("patches", [])
    if m.get("name") != name:
        raise SystemExit(f"{path}: name {m.get('name')!r} does not match file name {name!r}")
    if not isinstance(m.get("version"), str):
        raise SystemExit(f"{path}: version must be a quoted string")
    return m


def apply_exclude(m: dict, exclude: list[str]) -> None:
    """Remove the paths matching `exclude` from every rule of manifest `m`."""
    if not exclude:
        return
    kept = []
    for r in m.get("rules", []):
        if "path" in r or "paths" in r:
            paths = [x for x in (r.get("paths") or [r["path"]])
                     if not any(fnmatch.fnmatch(x, e) for e in exclude)]
            if not paths:
                continue
            r = {k: v for k, v in r.items() if k != "path"} | {"paths": paths}
        else:
            r = r | {"exclude": list(r.get("exclude", [])) + list(exclude)}
        kept.append(r)
    m["rules"] = kept


def all_names(root: Path) -> list[str]:
    d = root / DEPS_DIR
    return sorted(p.stem for p in d.glob("*.yaml") if not p.stem.endswith(".private"))


def vkey(v: str) -> tuple:
    """Order 1.10.2 after 1.9.0; a pre-release sorts before its release."""
    v = v.lstrip("v")
    core, _, pre = v.partition("-")
    nums = tuple(int(x) if x.isdigit() else 0 for x in re.split(r"[.+]", core))
    return nums + ((1,) if not pre else (0, pre))


# --------------------------------------------------------------------------
# file walking

def walk(root: Path, globs: list[str], exclude: list[str], include: list[str] | None = None) -> list[Path]:
    """Files under root matching `globs` by name or path, minus `exclude`;
    when `include` is given, only paths matching one of its globs."""
    out = []
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = [d for d in dirnames if d not in SKIP_DIRS]
        for fn in filenames:
            rel = os.path.relpath(os.path.join(dirpath, fn), root)
            if any(fnmatch.fnmatch(fn, g) or fnmatch.fnmatch(rel, g) for g in globs):
                if any(fnmatch.fnmatch(rel, e) for e in exclude):
                    continue
                if include and not any(fnmatch.fnmatch(rel, i) for i in include):
                    continue
                out.append(Path(rel))
    return sorted(out)


def lines(root: Path, rel: Path):
    try:
        text = (root / rel).read_text(errors="replace")
    except (OSError, UnicodeDecodeError):
        return
    for i, line in enumerate(text.splitlines(), 1):
        if any(mk in line for mk in ALLOW_MARKERS):
            continue
        yield i, line


@dataclass
class Hit:
    path: str
    line: int
    found: str
    want: str
    what: str
    span: tuple[int, int] = (0, 0)  # character offsets inside the line, for bump


@dataclass
class Report:
    failures: list[Hit] = field(default_factory=list)
    matched: list[Hit] = field(default_factory=list)  # places that agree


# --------------------------------------------------------------------------
# rules

PIP_GLOBS = ["requirements*.txt", "pyproject.toml", "*.sh", "*.yml", "*.yaml", "*.j2", "*.py",
             "Dockerfile", "Dockerfile.*", "*.toml", "*.cfg", "*.in", "*.go", "*.html", "*.ts", "*.js"]


def match_groups(m, r, rel, i, line, rx, rep):
    n = 0
    for mt in rx.finditer(line):
        n += 1
        for g, val in mt.groupdict().items():
            if val is None:
                continue
            want = m["version"] if g in ("version", "v") else m["facts"].get(g)
            if want is None:
                rep.failures.append(Hit(str(rel), i, val, "?", f"fact {g!r} is not in the manifest"))
                continue
            hit = Hit(str(rel), i, val, str(want), r.get("what", f"{g} in {rel.name}"), mt.span(g))
            (rep.matched if val == str(want) else rep.failures).append(hit)
    return n


def rule_text(root, m, r, rep):
    rx = re.compile(r["pattern"])
    for path in r.get("paths") or [r["path"]]:
        rel = Path(path)
        if not (root / rel).exists():
            rep.failures.append(Hit(str(rel), 0, "missing", "file exists", f"text rule: {path} is gone"))
            continue
        n = sum(match_groups(m, r, rel, i, line, rx, rep) for i, line in lines(root, rel))
        if n < int(r.get("min", 1)):
            rep.failures.append(Hit(str(rel), 0, f"{n} match(es)", f">= {r.get('min', 1)}",
                                    f"pattern not found: {r['pattern']}"))


def rule_regex(root, m, r, rep):
    """Like text, across every file matching `globs`; zero matches is fine."""
    rx = re.compile(r["pattern"])
    for rel in walk(root, r["globs"], r.get("exclude", []), r.get("include")):
        for i, line in lines(root, rel):
            match_groups(m, r, rel, i, line, rx, rep)


CARGO_DEP = re.compile(r'^\s*([A-Za-z0-9_-]+)\s*=\s*(.+)$')


def rule_cargo(root, m, r, rep):
    crx = re.compile(r["crate"])
    want = m["version"]
    for rel in walk(root, ["Cargo.toml"], r.get("exclude", []), r.get("include")):
        for i, line in lines(root, rel):
            mt = CARGO_DEP.match(line)
            if not mt or not crx.fullmatch(mt.group(1)):
                continue
            rhs = mt.group(2)
            if rhs.lstrip().startswith("{"):
                vm = re.search(r'version\s*=\s*"([^"]*)"', rhs)
                if not vm:
                    if "path" in rhs or "workspace" in rhs or "git" in rhs:
                        continue  # a path/workspace/git dependency carries no version here
                    rep.failures.append(Hit(str(rel), i, rhs.strip(), f"={want}", f"{mt.group(1)}: no version"))
                    continue
                val, off = vm.group(1), line.index(rhs) + vm.start(1)
            else:
                vm = re.match(r'\s*"([^"]*)"', rhs)
                if not vm:
                    continue
                val, off = vm.group(1), line.index(rhs) + vm.start(1)
            hit = Hit(str(rel), i, val, f"={want}", f"{mt.group(1)} must be an exact pin", (off, off + len(val)))
            (rep.matched if val == f"={want}" else rep.failures).append(hit)
    for rel in walk(root, ["Cargo.lock"], r.get("exclude", []), r.get("include")):
        text = (root / rel).read_text(errors="replace").splitlines()
        for i, line in enumerate(text):
            nm = re.match(r'name = "([^"]+)"', line)
            if nm and crx.fullmatch(nm.group(1)) and i + 1 < len(text):
                vm = re.match(r'version = "([^"]+)"', text[i + 1])
                if vm:
                    hit = Hit(str(rel), i + 2, vm.group(1), want, f"{nm.group(1)} in the lock file")
                    (rep.matched if vm.group(1) == want else rep.failures).append(hit)


def rule_regex_everywhere(root, m, r, rep, globs, rx, label):
    want = m["version"]
    for rel in walk(root, globs, r.get("exclude", []), r.get("include")):
        for i, line in lines(root, rel):
            for mt in rx.finditer(line):
                val = mt.group("v")
                hit = Hit(str(rel), i, val, want, label, mt.span("v"))
                (rep.matched if val == want else rep.failures).append(hit)


def rule_pip(root, m, r, rep):
    pkg = re.escape(r["package"])
    rx = re.compile(rf'(?<![A-Za-z0-9_-]){pkg}\s*==\s*(?P<v>[0-9][A-Za-z0-9.+-]*)')
    rule_regex_everywhere(root, m, r, rep, r.get("globs", PIP_GLOBS), rx, f"pip {r['package']}")


def rule_npm(root, m, r, rep):
    pkg = re.escape(r["package"])
    rx = re.compile(rf'"{pkg}"\s*:\s*"(?P<v>[^"]+)"')
    rule_regex_everywhere(root, m, r, rep, ["package.json"], rx, f"npm {r['package']} (exact)")


def rule_gomod(root, m, r, rep):
    mod = re.escape(r["module"])
    rx = re.compile(rf'{mod}\s+v(?P<v>[0-9][A-Za-z0-9.+-]*)')
    rule_regex_everywhere(root, m, r, rep, ["go.mod"], rx, f"go module {r['module']}")


def rule_forbid(root, m, r, rep):
    rx = re.compile(r["pattern"])
    for rel in walk(root, r.get("glob", ["*"]) if isinstance(r.get("glob"), list) else [r.get("glob", "*")],
                    r.get("exclude", [])):
        for i, line in lines(root, rel):
            if rx.search(line):
                rep.failures.append(Hit(str(rel), i, line.strip()[:120], "absent", r.get("what", "forbidden")))


RULES = {"text": rule_text, "regex": rule_regex, "cargo": rule_cargo, "pip": rule_pip, "npm": rule_npm,
         "gomod": rule_gomod, "forbid": rule_forbid}


def check_patches(root, m, rep):
    """Every carried patch file exists; every file in a patch dir is listed."""
    listed = set()
    for p in m["patches"]:
        f = p.get("file")
        if not f:
            continue
        listed.add(f)
        if not (root / f).exists():
            rep.failures.append(Hit(f, 0, "missing", "exists", "manifest lists a patch file that is gone"))
    for d in m.get("patch_dirs", []):
        for f in sorted((root / d).glob("*.patch")):
            rel = str(f.relative_to(root))
            if rel not in listed:
                rep.failures.append(Hit(rel, 0, "unlisted", "listed in manifest",
                                        "patch file carried but not in the manifest's patches"))


def run_check(root: Path, m: dict) -> Report:
    rep = Report()
    for r in m["rules"]:
        kind = r.get("kind")
        if kind not in RULES:
            rep.failures.append(Hit(m["_path"], 0, str(kind), "|".join(RULES), "unknown rule kind"))
            continue
        RULES[kind](root, m, r, rep)
    check_patches(root, m, rep)
    return rep


# --------------------------------------------------------------------------
# commands

def cmd_list(root, args):
    for n in all_names(root):
        m = load_manifest(root, n)
        live = [p for p in m["patches"] if not p.get("fixed_in")]
        print(f"{n:<16} {m['version']:<12} rules={len(m['rules']):<3} patches={len(m['patches'])} "
              f"(still needed {len(live)})  {m.get('upstream', {}).get('github', '')}")
    return 0


def cmd_check(root, args):
    names = args.names or all_names(root)
    bad = 0
    for n in names:
        m = load_manifest(root, n)
        rep = run_check(root, m)
        if rep.failures:
            bad += 1
            print(f"[FAIL] {n} {m['version']}: {len(rep.failures)} place(s) disagree with {m['_path']}")
            for h in rep.failures[:60]:
                print(f"  {h.path}:{h.line}: {h.what}: found {h.found!r}, want {h.want!r}")
            if len(rep.failures) > 60:
                print(f"  ... {len(rep.failures) - 60} more")
            print(f"  Change the version in one place: python3 ops/upstream/upstream.py bump {n} <version>")
        else:
            print(f"[ok]   {n} {m['version']}: {len(rep.matched)} place(s) agree with {m['_path']}")
    return 1 if bad else 0


def set_manifest_value(text: str, key: str, value: str, top: bool) -> str:
    """Rewrite `version: "X"` (top) or `  key: "X"` under facts:, keeping comments."""
    if top:
        new, n = re.subn(r'(?m)^(version:\s*)"[^"]*"', rf'\g<1>"{value}"', text, count=1)
    else:
        new, n = re.subn(rf'(?m)^(\s+{re.escape(key)}:\s*)"[^"]*"', rf'\g<1>"{value}"', text, count=1)
    if n != 1:
        raise SystemExit(f"could not rewrite {key} in the manifest (expected a quoted value)")
    return new


def cmd_bump(root, args):
    m = load_manifest(root, args.name)
    old, new = m["version"], args.version.lstrip("v")
    sets = dict(s.split("=", 1) for s in args.set or [])
    if "series" in m["facts"] and "series" not in sets:
        sets["series"] = ".".join(new.split(".")[:2])
    unknown = set(sets) - set(m["facts"])
    if unknown:
        raise SystemExit(f"--set names facts the manifest does not have: {sorted(unknown)}")
    # Every place that agrees with the current manifest is rewritten; places that
    # already disagree are left for the check to report, never silently fixed.
    rep = run_check(root, m)
    if rep.failures and not args.force:
        print("the tree already disagrees with the manifest; run `check` and fix that first "
              "(or pass --force to rewrite only the places that agree)")
        return 1
    edits: dict[str, list[tuple[int, tuple[int, int], str]]] = {}
    for h in rep.matched:
        if h.want in (old, f"={old}"):
            repl = h.want.replace(old, new)
        else:
            fact = next((k for k, v in m["facts"].items() if str(v) == h.want), None)
            if fact is None or fact not in sets:
                continue
            repl = sets[fact]
        if h.path.endswith("Cargo.lock"):
            continue  # lock files are regenerated by the manifest's `lockfiles` commands, never edited
        edits.setdefault(h.path, []).append((h.line, h.span, repl))
    changed = []
    for path, es in sorted(edits.items()):
        f = root / path
        ls = f.read_text().splitlines(keepends=True)
        for ln, (a, b), repl in sorted(es, key=lambda e: (e[0], -e[1][0])):
            line = ls[ln - 1]
            line = line[:a] + repl + line[b:]
            ls[ln - 1] = line
        if not args.dry_run:
            f.write_text("".join(ls))
        changed.append((path, len(es)))
    mtext = (root / m["_path"]).read_text()
    mtext = set_manifest_value(mtext, "version", new, top=True)
    for k, v in sets.items():
        mtext = set_manifest_value(mtext, k, v, top=False)
    if not args.dry_run:
        (root / m["_path"]).write_text(mtext)
    for path, n in changed:
        print(f"  {'would rewrite' if args.dry_run else 'rewrote'} {path} ({n} place(s))")
    print(f"{args.name}: {old} -> {new} in {len(changed)} file(s)" + (" (dry run)" if args.dry_run else ""))
    stale = [p for p in m["patches"] if p.get("fixed_in") and vkey(p["fixed_in"]) <= vkey(new)]
    for p in stale:
        print(f"  DROP patch {p['file']}: upstream #{p.get('upstream_pr')} shipped in {p['fixed_in']}; "
              f"delete it, its lines in the build file and its manifest entry")
    missing_facts = [k for k in m["facts"] if k not in sets and m.get("facts_move_with_version", True)]
    if missing_facts and not args.dry_run:
        print("  facts not set by --set (check whether the new release changes them): "
              + ", ".join(missing_facts))
    for cmd in m.get("lockfiles", []):
        cmd = cmd.replace("{old}", old).replace("{new}", new)
        if args.lockfiles and not args.dry_run:
            print(f"  running: {cmd}")
            if subprocess.run(cmd, shell=True, cwd=root).returncode != 0:
                print(f"  lock file command failed: {cmd}")
                return 1
        else:
            print(f"  lock file (run it, or pass --lockfiles): {cmd}")
    print("  then: python3 ops/upstream/upstream.py check " + args.name)
    return 0


def cmd_patches(root, args):
    m = load_manifest(root, args.name)
    src = Path(args.src).resolve()
    rc = 0
    for p in m["patches"]:
        f = p.get("file")
        if not f:
            print(f"  {p.get('id', '?')}: no patch file (tracked only)")
            continue
        fwd = subprocess.run(["git", "-C", str(src), "apply", "--check", str(root / f)],
                             capture_output=True, text=True)
        rev = subprocess.run(["git", "-C", str(src), "apply", "--check", "-R", str(root / f)],
                             capture_output=True, text=True)
        if fwd.returncode == 0:
            state = "applies (still needed)"
        elif rev.returncode == 0:
            state = "already in upstream (drop it)"
        else:
            state = "does not apply (rebase it)"
            rc = 1
        ref = f"#{p['upstream_pr']}" if p.get("upstream_pr") else (f"issue #{p['upstream_issue']}" if p.get("upstream_issue") else "none")
        print(f"  {Path(f).name}: {state}  [{p.get('finding', '') or '-'}, upstream {ref}]")
    return rc


# --------------------------------------------------------------------------
# watch (public upstream data only; scheduling and the issue live privately)

def gh_get(path: str):
    req = urllib.request.Request(f"https://api.github.com{path}",
                                 headers={"Accept": "application/vnd.github+json",
                                          "User-Agent": "goat-upstream-watch"})
    tok = os.environ.get("GITHUB_TOKEN") or os.environ.get("GH_TOKEN")
    if tok:
        req.add_header("Authorization", f"Bearer {tok}")
    with urllib.request.urlopen(req, timeout=30) as r:
        return json.load(r)


def http_json(url: str):
    req = urllib.request.Request(url, headers={"User-Agent": "goat-upstream-watch"})
    with urllib.request.urlopen(req, timeout=30) as r:
        return json.load(r)


def latest_of(source: dict) -> list[dict]:
    """Releases newer-first: [{tag, date, body, url}]."""
    if "github" in source:
        rel = gh_get(f"/repos/{source['github']}/releases?per_page=30")
        return [{"tag": r["tag_name"], "date": (r.get("published_at") or "")[:10],
                 "body": r.get("body") or "", "url": r["html_url"], "prerelease": r["prerelease"]}
                for r in rel if not r.get("draft")]
    if "pypi" in source:
        d = http_json(f"https://pypi.org/pypi/{source['pypi']}/json")
        vs = sorted(d["releases"], key=vkey, reverse=True)
        return [{"tag": v, "date": (d["releases"][v][0]["upload_time"][:10] if d["releases"][v] else ""),
                 "body": "", "url": f"https://pypi.org/project/{source['pypi']}/{v}/", "prerelease": False}
                for v in vs[:10]]
    if "npm" in source:
        d = http_json(f"https://registry.npmjs.org/{source['npm']}")
        vs = sorted(d.get("versions", {}), key=vkey, reverse=True)
        return [{"tag": v, "date": d.get("time", {}).get(v, "")[:10], "body": "",
                 "url": f"https://www.npmjs.com/package/{source['npm']}/v/{v}", "prerelease": "-" in v}
                for v in vs[:10]]
    if "docker" in source:
        d = http_json(f"https://hub.docker.com/v2/repositories/{source['docker']}/tags?page_size=50")
        tags = [t["name"] for t in d.get("results", []) if re.match(r"v?\d+\.\d+", t["name"])]
        return [{"tag": t, "date": "", "body": "", "url": "", "prerelease": "-" in t}
                for t in sorted(tags, key=vkey, reverse=True)[:10]]
    return []


def shipped_in(repo: str, pr: int, tags: list[str]) -> str | None:
    """Oldest tag in `tags` (newest-first) that contains the PR's merge commit."""
    try:
        p = gh_get(f"/repos/{repo}/pulls/{pr}")
    except urllib.error.HTTPError:
        return "not found"
    sha = p.get("merge_commit_sha")
    if not p.get("merged_at") or not sha:
        return "open" if p.get("state") == "open" else "closed unmerged"
    found = None
    for t in tags:  # newest first; stop at the first tag that lacks it
        try:
            c = gh_get(f"/repos/{repo}/compare/{t}...{sha}")
        except urllib.error.HTTPError:
            continue
        if c.get("status") in ("behind", "identical"):
            found = t
        else:
            break
    return found or "merged, unreleased"


def cmd_watch(root, args):
    out = []
    for n in args.names or all_names(root):
        m = load_manifest(root, n)
        up = m.get("upstream", {})
        cur = m["version"]
        entry = {"name": n, "version": cur, "sources": [], "patches": []}
        kws = [k.lower() for k in up.get("watch_keywords", [])]
        for src in [dict(up, name=n)] + up.get("companions", []):
            try:
                rels = latest_of(src)
            except (urllib.error.URLError, KeyError, ValueError) as e:
                entry["sources"].append({"name": src.get("name"), "error": str(e)})
                continue
            stable = [r for r in rels if not r["prerelease"]]
            newer = [r for r in stable if vkey(r["tag"]) > vkey(src.get("version", cur))]
            hits = sorted({k for r in newer for k in kws if k in r["body"].lower()})
            entry["sources"].append({"name": src.get("name"), "current": src.get("version", cur),
                                     "latest": stable[0]["tag"] if stable else None,
                                     "newer": [{"tag": r["tag"], "date": r["date"], "url": r["url"]} for r in newer],
                                     "keyword_hits": hits})
        if "github" in up:
            try:
                tags = [r["tag"] for r in latest_of(up) if not r["prerelease"]]
            except urllib.error.URLError:
                tags = []
            for p in m["patches"]:
                pr = p.get("upstream_pr")
                if not pr or p.get("fixed_in"):
                    continue
                entry["patches"].append({"file": p.get("file"), "finding": p.get("finding"), "upstream_pr": pr,
                                         "shipped_in": shipped_in(up["github"], int(pr), tags)})
        out.append(entry)
    if args.json:
        print(json.dumps(out, indent=2))
        return 0
    for e in out:
        print(f"## {e['name']} (ours {e['version']})")
        for s in e["sources"]:
            if "error" in s:
                print(f"- {s['name']}: could not read ({s['error']})")
                continue
            tail = f"; keywords in notes: {', '.join(s['keyword_hits'])}" if s["keyword_hits"] else ""
            print(f"- {s['name']}: ours {s['current']}, latest {s['latest']}, "
                  f"{len(s['newer'])} newer{tail}")
        for p in e["patches"]:
            print(f"- patch {p['file']} ({p['finding'] or '-'}): upstream #{p['upstream_pr']} -> {p['shipped_in']}")
    return 0


# --------------------------------------------------------------------------
# self-test: every rule kind must go red on a broken fixture

def cmd_self_test(root, args):
    with tempfile.TemporaryDirectory() as td:
        t = Path(td)
        (t / DEPS_DIR).mkdir(parents=True)
        (t / "a").mkdir()
        (t / "a/Cargo.toml").write_text('[dependencies]\nfoo = { version = "1.2", features = ["x"] }\nfoo-ext = "=1.2.0"\n')
        (t / "a/Cargo.lock").write_text('[[package]]\nname = "foo"\nversion = "1.1.0"\n')
        (t / "a/requirements.txt").write_text("pyfoo==1.1.0\n")
        (t / "a/package.json").write_text('{"dependencies": {"@x/foo": "^1.2.0"}}\n')
        (t / "a/go.mod").write_text("require example.com/foo v1.1.0\n")
        (t / "a/Dockerfile").write_text("ARG FOO=1.1.0\nFROM foo:1.2.0@sha256:bbb\nFROM retired/foo:1\n")
        (t / "a/x.patch").write_text("")
        man = {"name": "foo", "version": "1.2.0", "facts": {"digest": "sha256:aaa"},
               "patch_dirs": ["a"],
               "rules": [{"kind": "cargo", "crate": "foo(-ext)?"},
                         {"kind": "pip", "package": "pyfoo"},
                         {"kind": "npm", "package": "@x/foo"},
                         {"kind": "gomod", "module": "example.com/foo"},
                         {"kind": "text", "path": "a/Dockerfile", "pattern": r"ARG FOO=(?P<version>\S+)"},
                         {"kind": "text", "path": "a/Dockerfile", "pattern": r"foo:(?P<v>[^@]+)@(?P<digest>sha256:\w+)"},
                         {"kind": "forbid", "glob": "Dockerfile", "pattern": r"retired/foo:"}]}
        (t / DEPS_DIR / "foo.yaml").write_text(yaml.safe_dump(man))
        rep = run_check(t, load_manifest(t, "foo"))
        kinds = {h.what.split(" ")[0] for h in rep.failures}
        want = {"foo", "pip", "npm", "go", "version", "digest", "forbidden", "patch"}
        got = {w for w in want if any(h.what.startswith(w) for h in rep.failures)}
        missing = want - got
        if missing:
            print(f"[self-test FAIL] these rule kinds did not go red on the broken fixture: {sorted(missing)}")
            print("  failures seen:", sorted(kinds))
            return 1
        # a private overlay's exclude removes a place from every rule
        (t / DEPS_DIR / "foo.private.yaml").write_text(yaml.safe_dump({"name": "foo", "exclude": ["a/*"]}))
        left = [(h.path, h.what) for h in run_check(t, load_manifest(t, "foo")).failures
                if not h.what.startswith("patch")]
        if left:
            print("[self-test FAIL] a private exclude left places in the check:", left)
            return 1
        (t / DEPS_DIR / "foo.private.yaml").unlink()
        # bump round-trip on a clean fixture
        (t / "a/Cargo.toml").write_text('[dependencies]\nfoo = { version = "=1.2.0" }\n')
        (t / "a/Cargo.lock").unlink()
        (t / "a/requirements.txt").write_text("pyfoo==1.2.0\n")
        (t / "a/package.json").write_text('{"dependencies": {"@x/foo": "1.2.0"}}\n')
        (t / "a/go.mod").write_text("require example.com/foo v1.2.0\n")
        (t / "a/Dockerfile").write_text("ARG FOO=1.2.0\nFROM foo:1.2.0@sha256:aaa\n")
        man["patches"] = [{"file": "a/x.patch", "finding": "F-0", "upstream_pr": 1, "fixed_in": "1.3.0"}]
        (t / DEPS_DIR / "foo.yaml").write_text(yaml.safe_dump(man, default_style=None).replace(
            "version: 1.2.0", 'version: "1.2.0"').replace("digest: sha256:aaa", 'digest: "sha256:aaa"'))
        if run_check(t, load_manifest(t, "foo")).failures:
            print("[self-test FAIL] clean fixture does not pass:",
                  [(h.path, h.what) for h in run_check(t, load_manifest(t, "foo")).failures])
            return 1
        ns = argparse.Namespace(name="foo", version="1.3.0", set=["digest=sha256:ccc"], dry_run=False,
                                force=False, lockfiles=False)
        cmd_bump(t, ns)
        after = run_check(t, load_manifest(t, "foo"))
        if after.failures or "=1.3.0" not in (t / "a/Cargo.toml").read_text() \
                or "sha256:ccc" not in (t / "a/Dockerfile").read_text():
            print("[self-test FAIL] bump did not leave a clean tree:",
                  [(h.path, h.what, h.found) for h in after.failures])
            return 1
    print("[self-test ok] every rule kind fails its broken fixture; bump leaves a clean tree")
    return 0


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--root", default=None, help="tree to act on (default: git top level)")
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("list")
    c = sub.add_parser("check"); c.add_argument("names", nargs="*")
    b = sub.add_parser("bump"); b.add_argument("name"); b.add_argument("version")
    b.add_argument("--set", action="append", metavar="FACT=VALUE")
    b.add_argument("--dry-run", action="store_true"); b.add_argument("--force", action="store_true")
    b.add_argument("--lockfiles", action="store_true", help="run the manifest's lock-file commands")
    p = sub.add_parser("patches"); p.add_argument("name"); p.add_argument("--src", required=True)
    w = sub.add_parser("watch"); w.add_argument("names", nargs="*"); w.add_argument("--json", action="store_true")
    sub.add_parser("self-test")
    args = ap.parse_args(argv)
    root = Path(args.root) if args.root else Path(subprocess.run(
        ["git", "rev-parse", "--show-toplevel"], capture_output=True, text=True, check=True).stdout.strip())
    return {"list": cmd_list, "check": cmd_check, "bump": cmd_bump, "patches": cmd_patches,
            "watch": cmd_watch, "self-test": cmd_self_test}[args.cmd](root, args)


if __name__ == "__main__":
    sys.exit(main())
