"""Unpack a verified goat-xf-pipe-zenoh archive, and hold what is on disk to it.

usage: xf-pipe-unpack.py <archive> <top> <dest> <group>

Run as root by ops/ansible/goat-xf-pipe-deploy.yml on the host, after the play has
held <archive> to the SHA-256 its directory is named for. <dest> is
/opt/goat/xf-pipe/<sha256>; the archive's one top directory, <top>, is unpacked
under it. On every run:

  - every member is judged first: a plain file or directory only (no link, hard
    link, device or FIFO), a plain relative path under <top>/, no name twice;
  - the tree already at <dest>/<top> is compared with the archive: each path, its
    type, its owner's mode (files 0700 or 0600 and directories 0700 as unpacked
    here, owner root, group <group>; the play then gives that group, the
    service's, read and search, and anyone else nothing; no setuid, setgid or
    sticky bit) and each file's bytes. Anything else, a missing file or an extra
    one is a difference;
  - if it differs, or is not there, the archive is unpacked into a new directory
    beside it and renamed into place, so a run stopped halfway leaves nothing
    that looks complete;
  - <dest>'s parent and <dest> must be root's and writable by no one else.

Prints one JSON line: {"changed": bool, "files": n}. Exit 1 with one line on stderr
when it refuses.
"""
import hashlib
import json
import os
import shutil
import stat
import sys
import grp
import tarfile

MAX_MEMBERS, MAX_BYTES = 50000, 2 * 1024 * 1024 * 1024


def no(why):
    sys.stderr.write(why + "\n")
    sys.exit(1)


def held(path):
    """Root's, and writable by no one else; never a link."""
    st = os.lstat(path)
    if stat.S_ISLNK(st.st_mode) or st.st_uid != 0 or st.st_mode & 0o022:
        no("%s is not root's alone (a link, another owner, or writable by its group or others)" % path)


def members(archive, top):
    """{relative path: (kind, mode, sha256 or None)} of the archive, judged."""
    want, total = {}, 0
    try:
        with tarfile.open(archive, mode="r:gz") as tar:
            for n, m in enumerate(tar):
                if n >= MAX_MEMBERS:
                    no("the archive has more than %d members" % MAX_MEMBERS)
                name = m.name.rstrip("/")
                parts = name.split("/")
                if m.name.startswith("/") or any(p in ("", ".", "..") for p in parts) or any(ord(c) < 32 or c == "\\" for c in name):
                    no("the archive holds a path that is not a plain one: %r" % name[:80])
                if parts[0] != top:
                    no("the archive holds %r, outside its top directory %s" % (name[:80], top))
                if not (m.isreg() or m.isdir()) or m.issparse():
                    no("the archive holds %r, which is not a plain file or directory" % name[:80])
                rel = "/".join(parts[1:])
                if rel in want:
                    no("the archive names %r twice" % name[:80])
                if m.isdir():
                    want[rel] = ("d", 0o700, None)
                    continue
                total += m.size
                if total > MAX_BYTES:
                    no("the archive unpacks to more than %d bytes" % MAX_BYTES)
                h = hashlib.sha256()
                f = tar.extractfile(m)
                for chunk in iter(lambda: f.read(1 << 20), b""):
                    h.update(chunk)
                want[rel] = ("f", 0o700 if m.mode & 0o111 else 0o600, h.hexdigest())
    except (tarfile.TarError, OSError, EOFError) as e:
        no("it is not a gzipped tar archive: %s" % str(e)[:120])
    # The directories a member implies are the archive's too.
    for rel in list(want):
        while "/" in rel:
            rel = rel.rsplit("/", 1)[0]
            want.setdefault(rel, ("d", 0o700, None))
    want.setdefault("", ("d", 0o700, None))
    for rel in want:
        parent = rel.rsplit("/", 1)[0] if "/" in rel else ""
        if rel and parent and want.get(parent, ("d",))[0] != "d":
            no("the archive holds %r under a file" % rel[:80])
    return want


def owner_mode(mode):
    """The owner bits, when the group has at most those bits without write,
    others have none and no setuid, setgid or sticky bit is set (the play gives
    the service's group read, and search on directories); None otherwise."""
    u, g, o = (mode >> 6) & 7, (mode >> 3) & 7, mode & 7
    return (u << 6) if mode & 0o7000 == 0 and o == 0 and g & ~(u & 5) == 0 else None


def found(root, gid):
    """{relative path: (kind, mode, sha256 or None)} on disk, links not followed."""
    st = os.lstat(root)
    got = {"": ("d", owner_mode(stat.S_IMODE(st.st_mode)), None) if st.st_uid == 0 and st.st_gid == gid else ("owner", st.st_uid, None)}
    for base, dirs, names in os.walk(root):
        for n in dirs + names:
            p = os.path.join(base, n)
            rel = os.path.relpath(p, root)
            st = os.lstat(p)
            if st.st_uid != 0 or st.st_gid != gid:
                got[rel] = ("owner", st.st_uid, None)
            elif stat.S_ISDIR(st.st_mode):
                got[rel] = ("d", owner_mode(stat.S_IMODE(st.st_mode)), None)
            elif stat.S_ISREG(st.st_mode):
                with open(p, "rb") as fh:
                    h = hashlib.sha256()
                    for chunk in iter(lambda: fh.read(1 << 20), b""):
                        h.update(chunk)
                got[rel] = ("f", owner_mode(stat.S_IMODE(st.st_mode)), h.hexdigest())
            else:
                got[rel] = ("other", 0, None)
    return got


def make_dirs(into, path, gid):
    """path and each directory between into and it, one level at a time: root's,
    the group's, 0700. An archive may list only its files, so a directory it
    implies is made here and must carry the group as a listed one does."""
    cur = into
    for part in os.path.relpath(path, into).split(os.sep):
        cur = os.path.join(cur, part)
        if not os.path.lexists(cur):
            os.mkdir(cur, 0o700)
        os.chown(cur, 0, gid)
        os.chmod(cur, 0o700)


def unpack(archive, top, into, gid):
    os.makedirs(into, mode=0o700)
    with tarfile.open(archive, mode="r:gz") as tar:
        for m in tar:
            rel = "/".join(m.name.rstrip("/").split("/")[1:])
            p = os.path.join(into, top, rel) if rel else os.path.join(into, top)
            if m.isdir():
                make_dirs(into, p, gid)
                continue
            make_dirs(into, os.path.dirname(p), gid)
            mode = 0o700 if m.mode & 0o111 else 0o600
            fd = os.open(p, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
            with os.fdopen(fd, "wb") as out:
                shutil.copyfileobj(tar.extractfile(m), out)
            os.chown(p, 0, gid)
            os.chmod(p, mode)


def main(archive, top, dest, group):
    try:
        gid = grp.getgrnam(group).gr_gid
    except KeyError:
        no("there is no group %s" % group)
    want = members(archive, top)
    if "goat-xf-pipe-zenoh" not in want or want["goat-xf-pipe-zenoh"][:2] != ("f", 0o700):
        no("the archive has no program goat-xf-pipe-zenoh in %s" % top)
    parent = os.path.dirname(dest)
    os.makedirs(parent, mode=0o755, exist_ok=True)
    held(parent)
    if not os.path.lexists(dest):
        os.mkdir(dest, 0o700)
        os.chown(dest, 0, gid)
    held(dest)
    # What a run stopped halfway left behind.
    for n in os.listdir(dest):
        if n.startswith((".unpacking-", ".replaced-")):
            shutil.rmtree(os.path.join(dest, n), ignore_errors=True)
    tree = os.path.join(dest, top)
    same = os.path.isdir(tree) and not os.path.islink(tree) and found(tree, gid) == want
    if not same:
        fresh = os.path.join(dest, ".unpacking-%d" % os.getpid())
        shutil.rmtree(fresh, ignore_errors=True)
        try:
            unpack(archive, top, fresh, gid)
            if found(os.path.join(fresh, top), gid) != want:
                no("the tree unpacked from the archive is not the archive's")
            old = os.path.join(dest, ".replaced-%d" % os.getpid())
            if os.path.lexists(tree):
                os.rename(tree, old)
            os.rename(os.path.join(fresh, top), tree)
            shutil.rmtree(old, ignore_errors=True)
        finally:
            shutil.rmtree(fresh, ignore_errors=True)
    for n in os.listdir(dest):
        if n != top:
            no("%s holds %s, which is not this archive's; remove it and run again" % (dest, n))
    print(json.dumps({"changed": not same, "files": sum(1 for k in want.values() if k[0] == "f")}))


if __name__ == "__main__":
    main(*sys.argv[1:5])
