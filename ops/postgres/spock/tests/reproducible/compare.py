#!/usr/bin/env python3
"""Compare two OCI image tarballs layer by layer; list every differing file and how it differs.

Usage: compare.py A.tar B.tar
Exit 0 when every rootfs diff_id matches, 1 otherwise.
"""
import hashlib, io, json, sqlite3, subprocess, sys, tarfile, tempfile, os


def load(path):
    t = tarfile.open(path)
    blob = lambda d: t.extractfile("blobs/sha256/" + d.split(":")[1]).read()
    idx = json.loads(t.extractfile("index.json").read())
    m = json.loads(blob(idx["manifests"][0]["digest"]))
    if "manifests" in m:
        m = json.loads(blob(m["manifests"][0]["digest"]))
    cfg = json.loads(blob(m["config"]["digest"]))
    return cfg["rootfs"]["diff_ids"], [blob(l["digest"]) for l in m["layers"]], m["layers"], cfg


def members(raw, media):
    import gzip
    if raw[:2] == b"\x1f\x8b":
        raw = gzip.decompress(raw)
    elif raw[:4] == b"\x28\xb5\x2f\xfd":
        raw = subprocess.run(["zstd", "-dc"], input=raw, capture_output=True, check=True).stdout
    out = {}
    with tarfile.open(fileobj=io.BytesIO(raw)) as t:
        for i in t:
            data = t.extractfile(i).read() if i.isfile() else b""
            out[i.name] = (i, data)
    return out


def sqlite_dump(data):
    with tempfile.NamedTemporaryFile(suffix=".sqlite", delete=False) as f:
        f.write(data)
    try:
        c = sqlite3.connect(f.name)
        return list(c.iterdump())
    except Exception as e:
        return ["<cannot dump: %s>" % e]
    finally:
        os.unlink(f.name)


def main(a, b):
    da, la, ma, ca = load(a)
    db, lb, mb, cb = load(b)
    hist = [h.get("created_by", "") for h in ca.get("history", []) if not h.get("empty_layer")]
    bad = 0
    for n, (x, y) in enumerate(zip(da, db)):
        tag = hist[n][-90:] if n < len(hist) else ""
        print(f"layer {n}: {'SAME' if x == y else 'DIFF'} {x[7:19]} {y[7:19]}  {tag}")
        if x == y:
            continue
        bad += 1
        fa, fb = members(la[n], ma[n]["mediaType"]), members(lb[n], mb[n]["mediaType"])
        for name in sorted(set(fa) | set(fb)):
            if name not in fa or name not in fb:
                print(f"   only-in-{'A' if name in fa else 'B'}: {name}")
                continue
            (ia, xa), (ib, xb) = fa[name], fb[name]
            how = []
            if xa != xb:
                how.append(f"content sha {hashlib.sha256(xa).hexdigest()[:12]} vs {hashlib.sha256(xb).hexdigest()[:12]} size {len(xa)} vs {len(xb)}")
            for attr in ("mtime", "mode", "uid", "gid", "linkname", "type", "pax_headers"):
                if getattr(ia, attr) != getattr(ib, attr):
                    how.append(f"{attr} {getattr(ia, attr)!r} vs {getattr(ib, attr)!r}")
            if how:
                print(f"   {name}: " + "; ".join(how))
                if xa != xb and ("sqlite" in name or name.endswith(".db")):
                    sa, sb = set(sqlite_dump(xa)), set(sqlite_dump(xb))
                    for line in sorted(sa - sb)[:15]:
                        print(f"      A-only: {line[:220]}")
                    for line in sorted(sb - sa)[:15]:
                        print(f"      B-only: {line[:220]}")
                elif xa != xb and len(xa) < 20000 and b"\0" not in xa[:4096]:
                    import difflib
                    for line in list(difflib.unified_diff(xa.decode(errors="replace").splitlines(), xb.decode(errors="replace").splitlines(), lineterm="", n=0))[2:20]:
                        print(f"      {line[:220]}")
    print("RESULT:", "IDENTICAL" if bad == 0 else f"{bad} layer(s) differ")
    print("A diff_ids:", json.dumps(da))
    print("B diff_ids:", json.dumps(db))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main(*sys.argv[1:3]))
