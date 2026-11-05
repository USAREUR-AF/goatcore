#!/usr/bin/env python3
"""Give the packages a dnf install just added a fixed install time, so the layer is reproducible.

rpm 4.16 (Rocky 9) stamps every installed header with the wall clock: RPMTAG_INSTALLTIME
and RPMTAG_INSTALLTID inside /var/lib/rpm/rpmdb.sqlite, plus the Installtid index; dnf
writes the same clock into the `trans` rows of /var/lib/dnf/history.sqlite. Those
are the only bytes that differ between two builds of the same tree once dnf's logs
are kept out of the image. This rewrites them, for the packages and transactions the
install added, to STAMP and compacts both databases so no freed page keeps the old value.

Usage: pin-install-time.py mark            print the state to pass back after the install
       pin-install-time.py pin MARK [STAMP]
  MARK   "<highest rpmdb instance>:<highest dnf transaction id>" from `mark`.
  STAMP  the time to record; default SOURCE_DATE_EPOCH, else the newest install
         time already in the image (the base's), which is fixed by the base digest.

Run from a build bind mount, so the script itself never lands in a layer.
"""
import os, sqlite3, struct, sys

RPMDB = "/var/lib/rpm/rpmdb.sqlite"
HISTORY = "/var/lib/dnf/history.sqlite"
INSTALLTIME, INSTALLTID = 1008, 1128   # rpmtag.h; both RPM_INT32_TYPE


def tags(blob):
    """Yield (tag, data offset, count) for the INT32 entries of an on-disk rpm header blob."""
    il, dl = struct.unpack(">ii", blob[:8])
    base = 8 + 16 * il
    for i in range(il):
        tag, typ, off, cnt = struct.unpack(">iiii", blob[8 + 16 * i:24 + 16 * i])
        if typ == 4:
            yield tag, base + off, cnt


def get(blob, want):
    for tag, off, _ in tags(blob):
        if tag == want:
            return struct.unpack(">I", blob[off:off + 4])[0]
    return None


def put(blob, stamp):
    b = bytearray(blob)
    for tag, off, cnt in tags(blob):
        if tag in (INSTALLTIME, INSTALLTID):
            for k in range(cnt):
                b[off + 4 * k:off + 4 * k + 4] = struct.pack(">I", stamp)
    return bytes(b)


def compact(path):
    """Fold any WAL into the file, zero freed space, and leave no -wal/-shm beside it."""
    db = sqlite3.connect(path, isolation_level=None)
    wal = db.execute("PRAGMA journal_mode").fetchone()[0].lower() == "wal"
    if wal:
        db.execute("PRAGMA wal_checkpoint(TRUNCATE)")
    db.execute("VACUUM")
    if wal:
        db.execute("PRAGMA wal_checkpoint(TRUNCATE)")
    db.close()
    for side in ("-wal", "-shm"):
        try:
            os.unlink(path + side)
        except FileNotFoundError:
            pass   # sqlite already removed it on close


def mark():
    r = sqlite3.connect("file:%s?mode=ro" % RPMDB, uri=True).execute("SELECT max(hnum) FROM Packages").fetchone()[0]
    t = sqlite3.connect("file:%s?mode=ro" % HISTORY, uri=True).execute("SELECT max(id) FROM trans").fetchone()[0]
    print("%d:%d" % (r or 0, t or 0))


def main(cmd, marked=None, stamp=None):
    if cmd == "mark":
        mark()
        return
    top_hnum, top_trans = (int(x) for x in marked.split(":"))
    db = sqlite3.connect(RPMDB, isolation_level=None)
    db.execute("PRAGMA secure_delete = ON")
    rows = db.execute("SELECT hnum, blob FROM Packages").fetchall()
    times = {h: get(b, INSTALLTIME) or 0 for h, b in rows}
    if stamp is None:
        stamp = os.environ.get("SOURCE_DATE_EPOCH") or max(t for h, t in times.items() if h <= top_hnum)
    stamp = int(stamp)
    new = [h for h in times if h > top_hnum]
    if not new:
        sys.exit("pin-install-time: no package added after rpmdb instance %d; nothing to pin" % top_hnum)
    print("pin-install-time: rpm recorded install times %s" % sorted({times[h] for h in new}))
    db.execute("BEGIN")
    tids = set()
    for h, b in rows:
        if h in new:
            tids.add(get(b, INSTALLTID))
            db.execute("UPDATE Packages SET blob = ? WHERE hnum = ?", (put(b, stamp), h))
    for t in tids:   # the index key is the tid in host byte order
        db.execute("UPDATE Installtid SET key = ? WHERE key = ?", (struct.pack("=I", stamp), struct.pack("=I", t)))
    db.execute("COMMIT")
    db.close()
    compact(RPMDB)

    h = sqlite3.connect(HISTORY, isolation_level=None)
    h.execute("PRAGMA secure_delete = ON")
    n = h.execute("UPDATE trans SET dt_begin = ?, dt_end = ? WHERE id > ?", (stamp, stamp, top_trans)).rowcount
    h.close()
    compact(HISTORY)
    print("pin-install-time: %d package(s), %d dnf transaction(s) stamped %d" % (len(new), n, stamp))


if __name__ == "__main__":
    main(*sys.argv[1:4])
