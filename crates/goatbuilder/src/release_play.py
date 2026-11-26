"""Unpack the play a release archive carries, and hold what is on disk to it.

usage: release_play.py <dest> <play> <ansible dir>   (the archive on stdin)

<play> is its path inside the archive's one top directory. Only the play's own
directory is unpacked. It gets this checkout's group_vars, host_vars and roles as
links beside the play, because ansible reads them from a play's directory; an
archive that carries its own, or a role of the same name, is refused. Run by
src/release_play.rs with the bytes it verified; prints the play's path, or one
line saying why not on stderr (exit 1). Never writes through a link.
"""
import io
import os
import shutil
import sys
import tarfile
import tempfile

MAX_MEMBERS, MAX_BYTES = 20000, 256 * 1024 * 1024
# What Ansible loads from beside a play or inside a role as code of its own:
# modules, module utilities, plugins of every kind and collections. A release's
# play runs goatcore's roles with goatcore's modules; it brings none in their place.
CODE_DIRS = ("library", "module_utils", "collections")


def no(why):
    sys.stderr.write(why + "\n")
    sys.exit(1)


def members(data, play):
    """{path under the play's directory: bytes}, after every member is judged."""
    top, files, dirs, total = None, {}, set(), 0
    at = os.path.dirname(play) + "/" if "/" in play else ""
    try:
        with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as tar:
            n = 0
            for m in tar:
                n += 1
                if n > MAX_MEMBERS:
                    no("the archive has more than %d members" % MAX_MEMBERS)
                parts = m.name.rstrip("/").split("/")
                if m.name.startswith("/") or any(p in ("", ".", "..") for p in parts) or any(ord(c) < 32 or c == "\\" for c in m.name):
                    no("the archive holds a path that is not a plain one: %r" % m.name[:80])
                if not (m.isreg() or m.isdir()) or m.issparse():
                    no("the archive holds %r, which is not a plain file or directory" % m.name[:80])
                top = top or parts[0]
                if parts[0] != top:
                    no("the archive has more than one top directory")
                rel = "/".join(parts[1:])
                if m.isdir():
                    dirs.add(rel)
                    continue
                if not rel:
                    no("the archive has a file and no top directory")
                total += m.size
                if total > MAX_BYTES:
                    no("the archive unpacks to more than %d bytes" % MAX_BYTES)
                if rel in files:
                    no("the archive names %r twice" % rel[:80])
                files[rel] = tar.extractfile(m).read()
    except (tarfile.TarError, OSError, EOFError, ValueError) as e:
        no("it is not a gzipped tar archive: %s" % str(e)[:120])
    if play not in files:
        no("the archive has no file %s" % play)
    # Two names that are one on a file system that ignores case.
    seen = {}
    for f in sorted(set(files) | dirs):
        if seen.setdefault(f.lower(), f) != f:
            no("the archive names %r and %r, which are one name where case is ignored" % (seen[f.lower()][:80], f[:80]))
    # A file that is also a directory of another file.
    for f in files:
        while "/" in f:
            f = f.rsplit("/", 1)[0]
            if f in files:
                no("the archive holds %r as a file and a directory" % f[:80])
    return {f[len(at):]: b for f, b in files.items() if f.startswith(at)}


def links(files, ansible):
    """{path: target} of this checkout's own, beside the play."""
    out = {}
    for f in files:
        parts = f.split("/")
        # Beside the play, and inside one of its roles.
        at = [parts[0]] + ([parts[2]] if parts[0] == "roles" and len(parts) > 2 else [])
        bad = [p for p in at if p in CODE_DIRS or p.endswith("_plugins")]
        if bad:
            no("the play brings Ansible code of its own (%s): a release's play uses goatcore's modules and plugins" % f[:80])
    for d in ("group_vars", "host_vars"):
        if any(f == d or f.startswith(d + "/") for f in files):
            no("the play brings its own %s: a release adds no variables to a goatnet's" % d)
        if os.path.isdir(os.path.join(ansible, d)):
            out[d] = os.path.join(ansible, d)
    roles = os.path.join(ansible, "roles")
    own = {f.split("/")[1] for f in files if f.startswith("roles/")}
    for r in sorted(os.listdir(roles)) if os.path.isdir(roles) else []:
        if not os.path.isdir(os.path.join(roles, r)):
            continue
        if r in own:
            no("it carries a role %s, which goatcore has: a release adds roles and replaces none" % r)
        out["roles/" + r] = os.path.join(roles, r)
    return out


def found(dest):
    """What is on disk: {path: bytes} and {path: target}, links not followed."""
    f, l = {}, {}
    for base, dirs, names in os.walk(dest):
        for n in dirs + names:
            p = os.path.join(base, n)
            rel = os.path.relpath(p, dest)
            if os.path.islink(p):
                l[rel] = os.readlink(p)
            elif n in names:
                if not os.path.isfile(p):
                    no("%s holds %s, which is not a plain file; remove it and run again" % (dest, rel))
                with open(p, "rb") as fh:
                    f[rel] = fh.read()
    return f, l


def main(dest, play, ansible):
    files = members(sys.stdin.buffer.read(), play)
    want = links(files, ansible)
    if not os.path.lexists(dest):
        parent = os.path.dirname(dest)
        os.makedirs(parent, mode=0o700, exist_ok=True)
        tmp = tempfile.mkdtemp(dir=parent, prefix=".unpack-")
        try:
            for rel, b in files.items():
                p = os.path.join(tmp, rel)
                os.makedirs(os.path.dirname(p), mode=0o700, exist_ok=True)
                with open(p, "xb") as fh:
                    fh.write(b)
            for rel, target in want.items():
                p = os.path.join(tmp, rel)
                os.makedirs(os.path.dirname(p), mode=0o700, exist_ok=True)
                os.symlink(target, p)
            os.rename(tmp, dest)
        except OSError as e:
            # Another run unpacked it first, and what is there is judged below; or
            # it could not be written, and nothing of it is left.
            shutil.rmtree(tmp, ignore_errors=True)
            if not os.path.lexists(dest):
                no("the play could not be unpacked in %s: %s" % (parent, str(e)[:120]))
    if os.path.islink(dest) or not os.path.isdir(dest) or os.stat(dest).st_uid != os.getuid():
        no("%s is not a directory of this user's; remove it and run again" % dest)
    f, l = found(dest)
    if f != files:
        off = sorted(set(f) ^ set(files)) or sorted(k for k in f if f[k] != files[k])
        no("%s is not the archive's (%s); remove it and run again" % (dest, off[0][:80]))
    if l != want:
        off = sorted(set(l) ^ set(want)) or sorted(k for k in l if l[k] != want[k])
        # This checkout's own roles and variables moved on; the play did not.
        for rel in l:
            os.unlink(os.path.join(dest, rel))
        for rel, target in want.items():
            os.makedirs(os.path.dirname(os.path.join(dest, rel)), mode=0o700, exist_ok=True)
            os.symlink(target, os.path.join(dest, rel))
    print(os.path.join(dest, os.path.basename(play)))


if __name__ == "__main__":
    main(*sys.argv[1:4])
