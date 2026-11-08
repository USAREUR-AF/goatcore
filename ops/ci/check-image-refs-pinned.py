#!/usr/bin/env python3
"""check-image-refs-pinned.py — the ADR 0703 digest rule, past group_vars.

Called by ops/ci/check-no-floating-image-pins.sh, which covers hand-authored
group_vars. This covers the three places a floating tag still reached a build
or a host once that gate existed:

  role-default    ops/ansible/roles/*/defaults/*.yml  `*_image:` values. With no
                  group_vars override this IS what deploys, and 40 of them
                  carried a tag and no digest on 2026-09-22 — admit-audit on
                  `:latest`, so a rollback had no image to go back to.
  compose         `image:` literals in every compose file and compose template.
                  Test and proof stacks are included: a test that pulls a
                  moving tag stops proving the same thing week to week.
  dockerfile      `FROM` lines. These do not affect rollback (a rollback
                  redeploys a built image), they make a rebuild of the same
                  commit produce a different image.

A Dockerfile that DOES pin a digest must have its directory in
.github/dependabot.yml, which opens a PR when the pinned tag has a newer build.
Without it a pin trades silent drift for silent staleness (upstream CVE
rebuilds never arrive).

A reference passes when it carries `@sha256:`, is computed (jinja or `${…}` —
the variable it reads is what gets checked), names an earlier build stage, or
is `scratch`. A role default also passes when a sibling `<var>_digest` holds
the digest and the role verifies it (mediamtx).

Everything that floated when this guard landed is listed in
ops/ci/floating-image-pins.allow. That list only SHRINKS: a new floating
reference fails, and so does an allow line whose reference is now pinned or
gone, so a fix cannot leave its line behind. A repository layered onto this
one (goatstuff, a deployment overlay) lists its own files' entries, in the
same format and under the same rule, in
ops/ci/floating-image-pins.allow.d/*.allow.

  --seed   print allow-file lines for every current offender, for the one-time
           baseline. Never use it to wave a new offender through.
"""
import os
import re
import subprocess
import sys

ALLOW = "ops/ci/floating-image-pins.allow"
SKIP_DIRS = {".git", "node_modules", "target", ".venv", "vendor", "dist"}
# Image-adjacent keys that are not image references.
NOT_A_REF = re.compile(
    r"_image_(platform|digest|label|tiers|pull_policy|pull|name|tag|repo|arch|id|build|src|dir|path)$"
)
DIGEST = "@sha256:"
# A reviewed public copy of a file (ops/oss/export.py swaps it in for the
# original on export) is judged as the original it replaces: it shares the
# original's allow line and its dependabot directory, and adds neither.
PUBLIC_COPY = "ops/oss/docs-public/"


def original(p):
    return p[len(PUBLIC_COPY):] if p.startswith(PUBLIC_COPY) else p


def _tracked():
    """Tracked files when this is a git checkout, so a local run sees what CI
    sees and not build output or scratch files. None outside git (self-test).

    An assembled tree (.goat-assembled at its root) holds an overlay copied
    uncommitted over goatcore's HEAD, so the overlay's files are untracked
    there: it is read as tracked plus untracked, ignored files left out, or the
    overlay's own references and its allow.d lines would go unjudged."""
    cmd = ["git", "ls-files", "-z"]
    if os.path.isfile(".goat-assembled"):
        cmd += ["-co", "--exclude-standard"]
    try:
        r = subprocess.run(cmd, capture_output=True, check=True)
    except (OSError, subprocess.CalledProcessError):
        return None
    return sorted(set(x for x in r.stdout.decode().split("\0") if x))


TRACKED = _tracked()


def walk(root):
    if TRACKED is not None:
        pre = "" if root == "." else root.rstrip("/") + "/"
        yield from (f for f in TRACKED if f.startswith(pre))
        return
    for d, dirs, files in os.walk(root):
        dirs[:] = sorted(x for x in dirs if x not in SKIP_DIRS)
        for f in sorted(files):
            yield os.path.relpath(os.path.join(d, f), ".")


def clean(v):
    v = re.sub(r"\s+#.*$", "", v).strip()
    return v.strip("'\"").strip()


def computed(v):
    return "{{" in v or "$" in v


def role_defaults():
    out = []
    base = "ops/ansible/roles"
    if not os.path.isdir(base):
        return out
    for p in walk(base):
        parts = p.split(os.sep)
        if len(parts) < 6 or parts[4] != "defaults" or not p.endswith((".yml", ".yaml")):
            continue
        with open(p, encoding="utf-8", errors="replace") as source_file:
            lines = source_file.read().splitlines()
        keys = {}
        for ln in lines:
            m = re.match(r"^([a-z0-9_]+):\s*(.*)$", ln)
            if m:
                keys[m.group(1)] = clean(m.group(2))
        for n, ln in enumerate(lines, 1):
            m = re.match(r"^([a-z0-9_]*image[a-z0-9_]*):\s*(.*)$", ln)
            if not m or NOT_A_REF.search(m.group(1)):
                continue
            var, val = m.group(1), clean(m.group(2))
            if val in ("", "null", "~", "true", "false", "[]", "{}") or val.startswith(("[", "{", "|", ">")) and "{{" not in val:
                continue
            if DIGEST in val:
                continue
            # PENDING is the fail-closed placeholder: a role that carries it
            # refuses to deploy, so no image runs from it.
            if val == "PENDING":
                continue
            # A value that is only a jinja expression reads a pin from somewhere
            # else; that somewhere is what gets checked. A literal repo with a
            # jinja segment (ghcr.io/{{ ghcr_org }}/x:latest) is still a tag.
            if re.fullmatch(r"\{\{[^}]*\}\}", val):
                continue
            if keys.get(var + "_digest", "").startswith("sha256:"):
                continue
            out.append((p, n, var, val, "role-default"))
    return out


def compose_files():
    out = []
    for p in walk("."):
        b = os.path.basename(p)
        if "compose" not in b or not b.endswith((".yml", ".yaml", ".j2")):
            continue
        with open(p, encoding="utf-8", errors="replace") as source_file:
            for n, ln in enumerate(source_file, 1):
                m = re.match(r"^\s*image:\s*(\S.*)$", ln)
                if not m:
                    continue
                val = clean(m.group(1))
                if not val or DIGEST in val or computed(val):
                    continue
                out.append((p, n, val, val, "compose"))
    return out


DEPENDABOT = ".github/dependabot.yml"


def _dependabot_files():
    """This tree's dependabot file, and in an assembled tree each layer's own:
    a layer's directories are refreshed by its repository's dependabot, which
    the assembled tree does not carry (the marker names each layer's checkout
    as <layer>_path)."""
    files = [DEPENDABOT]
    if os.path.isfile(".goat-assembled"):
        with open(".goat-assembled", encoding="utf-8") as marker:
            for ln in marker:
                k, _, v = ln.strip().partition("=")
                if k.endswith("_path") and v:
                    files.append(os.path.join(v, DEPENDABOT))
    return [f for f in files if os.path.exists(f)]


def dependabot_dirs():
    dirs = set()
    for path in _dependabot_files():
        with open(path, encoding="utf-8") as source_file:
            for ln in source_file:
                m = re.match(r"^\s*(?:-|directory:)\s*[\"']?(/[^\s\"'#]*)", ln)
                if m:
                    dirs.add(m.group(1).rstrip("/") or "/")
    return dirs


def unrefreshed_pins():
    """Dockerfiles with a digest-pinned FROM whose directory dependabot skips."""
    out = []
    listed = dependabot_dirs()
    for p in walk("."):
        b = os.path.basename(p)
        if not (b == "Dockerfile" or b.startswith("Dockerfile.") or b.endswith(".Dockerfile")):
            continue
        with open(p, encoding="utf-8", errors="replace") as source_file:
            for n, ln in enumerate(source_file, 1):
                if re.match(r"^\s*FROM\s", ln, re.I) and DIGEST in ln:
                    d = "/" + os.path.dirname(original(p))
                    if d.rstrip("/") not in listed and (d.rstrip("/") or "/") not in listed:
                        out.append((p, n, d))
                    break
    return out


def dockerfiles():
    out = []
    for p in walk("."):
        b = os.path.basename(p)
        if not (b == "Dockerfile" or b.startswith("Dockerfile.") or b.endswith(".Dockerfile")):
            continue
        stages = set()
        with open(p, encoding="utf-8", errors="replace") as source_file:
            for n, ln in enumerate(source_file, 1):
                m = re.match(r"^\s*FROM\s+(.*)$", ln, re.I)
                if not m:
                    continue
                toks = [t for t in m.group(1).split() if not t.startswith("--")]
                if not toks:
                    continue
                img = toks[0]
                if len(toks) >= 3 and toks[1].lower() == "as":
                    stages.add(toks[2].lower())
                if img.lower() in stages or img == "scratch" or DIGEST in img or computed(img):
                    continue
                out.append((p, n, img, img, "dockerfile"))
    return out


def allow_files():
    """The allow list and its layered extensions, in a fixed order."""
    files = [ALLOW] if os.path.exists(ALLOW) else []
    ext = ALLOW + ".d"
    if os.path.isdir(ext):
        files += [os.path.join(ext, f) for f in sorted(os.listdir(ext)) if f.endswith(".allow")]
    return files


def load_allow():
    """(path, key) -> (allow file, line number)."""
    allow = {}
    for allow_file in allow_files():
        with open(allow_file, encoding="utf-8") as source_file:
            for n, ln in enumerate(source_file, 1):
                body = ln.split("#", 1)[0].strip()
                if not body:
                    continue
                path, _, key = body.partition(" ")
                allow.setdefault((path, key.strip()), (allow_file, n))
    return allow


def main():
    found = role_defaults() + compose_files() + dockerfiles()
    if "--seed" in sys.argv:
        for key in sorted({(p, k) for p, _, k, _, _ in found}):
            print(f"{key[0]} {key[1]}   # 2026-09-22 seed")
        return 0

    allow = load_allow()
    seen = set()
    fail = 0
    for p, n, key, val, kind in found:
        seen.add((original(p), key))
        if (original(p), key) in allow:
            continue
        fail += 1
        print(f"FLOATING {kind.upper():13} {p}:{n}")
        print(f"    {val}")
        print("    A tag is mutable: whoever can push can change what this builds or runs, with no commit.")
        print("    Pin the multi-arch index digest (tag@sha256:…). `crane digest <ref>` or")
        print("    `docker buildx imagetools inspect <ref>` prints it. Read the digest the")
        print("    host is RUNNING when pinning a deployed image, or the pin is an upgrade.")
    files = allow_files()
    for (p, key), (af, n) in sorted(allow.items(), key=lambda x: (files.index(x[1][0]), x[1][1])):
        if (p, key) not in seen:
            fail += 1
            print(f"STALE ALLOW   {af}:{n}")
            print(f"    {p} {key}")
            print("    This reference is pinned or gone. Delete the line — the list only shrinks.")
    for p, n, d in unrefreshed_pins():
        fail += 1
        print(f"NO REFRESH    {p}:{n}")
        print(f"    pins a base by digest, and {d} is not in {DEPENDABOT}.")
        print("    Add the directory under the docker ecosystem's `directories:` so the pin")
        print("    gets a PR when upstream rebuilds; a pin nobody refreshes goes stale silently.")
    if fail:
        print(f"\n{fail} problem(s). Floating references known on 2026-09-22 are listed in {ALLOW};")
        print("pin one and delete its line. Do not add lines.")
        return 1
    print(f"ok: {len(found)} floating reference(s), every one on the shrink-only list; no new ones")
    return 0


if __name__ == "__main__":
    sys.exit(main())
