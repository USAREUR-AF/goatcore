#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""The source of record of the tree a deploy runs from: one clone, or an
assembled tree (a goatcore checkout with an environment overlay copied over it).

An assembled tree carries a marker file, `.goat-assembled`, written by the
overlay's assemble step:

    goatcore_ref=<the goatcore commit in goatcore.ref, 40 hex>
    overlay_commit=<the overlay commit copied over it>[-dirty]
    goatstuff_ref=<the goatstuff commit in goatstuff.ref, 40 hex>   (optional)
    overlay_path=<the overlay checkout it was copied from>   (optional;
                                                               overlay_dir= also read)
    goatstuff_path=<the goatstuff checkout it was copied from> (optional)
    embargo=<id>/core.patch:<sha256>,<id>/stuff.patch:<sha256>,…  (optional)

Its source of record is the pair (goatcore commit, overlay commit). The tree is
current when its git HEAD is the goatcore commit goatcore.ref declares, every
overlay file in it is byte-for-byte the overlay commit's, no goatcore file under
ops/ was changed in it, and that overlay commit is not behind the overlay's main
in anything but docs/. Without the marker the tree is a clone, judged by
fresh-source.sh against its trunk remote as before.

With goatstuff_ref the assemble step layered goatstuff (at the commit the
overlay declares in goatstuff.ref) over goatcore before the overlay. The tree is
then current only if goatstuff.ref, when present, names that commit, and every
goatstuff file under ops/ the overlay does not replace is in the tree byte for
byte as that commit has it. Without goatstuff_ref nothing about goatstuff is
checked or recorded.

Pre-disclosure patches. An overlay may carry fixes that are not yet public as
patches, embargo/<id>/core.patch (applied to goatcore) and embargo/<id>/stuff.patch
(applied to goatstuff); the assemble step applies them, in id order, after
goatstuff and before the overlay, and records each in the embargo= line with
the sha256 of its bytes. The tree is then current only if the marker lists
exactly the patches the overlay commit carries, with those digests, and every
file the patches change is in the tree byte for byte as the pinned commit plus
the patches has it (a file a patch deletes is absent). Those files are judged
against that and nothing else; any other change to a goatcore or goatstuff file
is still refused. Without the embargo= line and with no patches in the overlay
commit, nothing about patches is checked or recorded.

    source_of_record.py kind     --repo DIR   prints "assembled" or "clone"
    source_of_record.py check    --repo DIR [--offline] [--quiet]
        contract C3 for an assembled tree. Exit 0 (current or ahead), 1 (refused:
        GB-064 behind or not what the marker says, GB-065 cannot be judged),
        2 (usage). Prints the goatcore commit on stdout, the value the drivers
        pin in GOAT_FRESH_SOURCE_TRUNK. With GOAT_FRESH_SOURCE_TRUNK set it
        fetches nothing and re-checks the tree against the pinned commit.
    source_of_record.py describe --repo DIR
        one JSON object for the deploy ledger row's `source_tree`:
        clone      {"commit": <HEAD>}
        assembled  {"goatcore": <commit>, "goatcore_ref": <goatcore.ref>,
                    "overlay": <commit>, "goatcore_export_of": <commit>?,
                    "goatstuff": <commit>?, "goatstuff_ref": <goatstuff.ref>?,
                    "embargo": [<id>, …]?}

Overlay checkout: the marker's overlay_path (or overlay_dir), else
$GOAT_OVERLAY_DIR. Overlay trunk remote: $GOAT_OVERLAY_TRUNK_REMOTE, else
$GOAT_OVERLAY_REMOTE, else `origin`. goatstuff checkout: the marker's
goatstuff_path, else $GOAT_GOATSTUFF_DIR. Standard library only.
"""
import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile

MARKER = ".goat-assembled"
# What the overlay's assemble step does not copy (it is not part of the tree).
NOT_COPIED = {"assemble.sh", "README.md"}
# Overlay paths that change no deploy: an overlay commit touching only these does
# not make an assembled tree stale.
NOT_DEPLOYED = [":(exclude)docs", ":(exclude)README.md", ":(exclude)assemble.sh"]
# Where a goatcore file edited inside the assembled tree changes a deploy.
CORE_DEPLOYED = ["ops", "goatcore.ref"]
SHA = re.compile(r"^[0-9a-f]{40}$")
# A pre-disclosure patch in the overlay: embargo/<id>/core.patch or embargo/<id>/stuff.patch.
PATCH = re.compile(r"^embargo/([A-Za-z0-9][A-Za-z0-9._-]*)/(core|stuff)\.patch$")


def git(repo, *args, env=None, raw=False):
    p = subprocess.run(["git", "-C", repo, *args], capture_output=True, text=True, env=env)
    return p.returncode, (p.stdout if raw else p.stdout.strip()), p.stderr.strip()


def marker(repo):
    path = os.path.join(repo, MARKER)
    if not os.path.isfile(path):
        return None
    out = {}
    with open(path, encoding="utf-8") as fh:
        for line in fh:
            line = line.strip()
            if not line or line.startswith("#") or "=" not in line:
                continue
            k, v = line.split("=", 1)
            out[k.strip()] = v.strip()
    return out


def say(msg):
    print("fresh-source: " + msg, file=sys.stderr)


def refuse(code, what, fixes):
    say(f"{code} REFUSE — {what}; refusing before any host is touched (contract C3).")
    for f in fixes:
        print(f"  {f}", file=sys.stderr)
    sys.exit(1)


def overlay_dir(m):
    d = m.get("overlay_path") or m.get("overlay_dir") or os.environ.get("GOAT_OVERLAY_DIR") or ""
    return os.path.abspath(d) if d else ""


def goatstuff_dir(m):
    d = m.get("goatstuff_path") or os.environ.get("GOAT_GOATSTUFF_DIR") or ""
    return os.path.abspath(d) if d else ""


def read_ref(repo, name):
    """The commit a <name> declaration file in the tree names, or None when there is none."""
    path = os.path.join(repo, name)
    if not os.path.isfile(path):
        return None
    with open(path, encoding="utf-8") as fh:
        return fh.read().strip()


def blobs(od, commit, prefixes=None):
    """{path: blob} for the regular files of commit (under prefixes, when given)."""
    rc, out, err = git(od, "ls-tree", "-r", "-z", "--full-tree", commit, *(prefixes or []))
    if rc != 0:
        return None, err
    want = {}
    for rec in out.split("\0"):
        if not rec:
            continue
        meta, path = rec.split("\t", 1)
        mode, kind, blob = meta.split()
        if kind != "blob" or mode == "120000":
            continue  # symlinks and submodules: not compared
        want[path] = blob
    return want, ""


def differing(repo, want):
    """Paths of want missing from the tree or whose bytes there are not the wanted blob."""
    paths = sorted(want)
    missing = [p for p in paths if not os.path.isfile(os.path.join(repo, p))]
    present = [p for p in paths if p not in set(missing)]
    got = []
    if present:
        p = subprocess.run(["git", "hash-object", "--no-filters", "--stdin-paths"], cwd=repo,
                           input="\n".join(present) + "\n", capture_output=True, text=True)
        if p.returncode != 0:
            return None, p.stderr.strip()
        got = p.stdout.split()
    return missing + [path for path, h in zip(present, got) if want[path] != h], ""


def export_of(repo):
    rc, msg, _ = git(repo, "log", "-1", "--format=%B", "HEAD")
    hit = re.search(r"export from the source tree at ([0-9a-f]{7,40})", msg or "")
    return hit.group(1) if hit else ""


def tree_matches_overlay(repo, od, commit):
    """Overlay files whose assembled bytes differ from the overlay commit's, or are missing."""
    want, err = blobs(od, commit)
    if want is None:
        return None, err
    for p in NOT_COPIED:
        want.pop(p, None)  # the assemble step's own files are not part of the tree
    return differing(repo, want)


def tree_matches_goatstuff(repo, sd, commit, overlay_paths, patched_paths=()):
    """goatstuff files under ops/ the overlay does not replace whose assembled bytes differ from the commit's, or are missing."""
    want, err = blobs(sd, commit, ["ops"])
    if want is None:
        return None, err
    for p in overlay_paths:
        want.pop(p, None)  # copied after goatstuff: the overlay's bytes win, checked against the overlay
    for p in patched_paths:
        want.pop(p, None)  # changed by a pre-disclosure patch: judged against the patched result
    return differing(repo, want)


def stray_core_edits(repo, overlay_paths):
    """goatcore files under CORE_DEPLOYED changed or added in this tree that the overlay did not supply."""
    rc, out, err = git(repo, "status", "--porcelain", "-z", "--untracked-files=all", "--", *CORE_DEPLOYED, raw=True)
    if rc != 0:
        return None, err
    stray, recs, i = [], out.split("\0"), 0
    while i < len(recs):
        rec = recs[i]; i += 1
        if len(rec) < 4:
            continue
        xy, path = rec[:2], rec[3:]
        if "R" in xy or "C" in xy:
            i += 1  # the rename's source path follows
        if path not in overlay_paths:
            stray.append(path)
    return stray, ""


def embargo_entries(m):
    """[(embargo/<id>/<core|stuff>.patch, sha256)] from the marker's embargo= line, in its order."""
    out = []
    for item in (m.get("embargo") or "").split(","):
        item = item.strip()
        if not item:
            continue
        path, _, digest = item.rpartition(":")
        out.append(("embargo/" + path, digest))
    return out


def embargo_ids(entries):
    return sorted({p.split("/")[1] for p, _ in entries if p.count("/") >= 2})


def patched(base, commit, patches):
    """{path: blob, or None when deleted} after applying patches, in order, to commit in base.

    The patches are applied to a scratch index with a scratch object store (base's
    objects borrowed read-only), so nothing in base changes.
    """
    with tempfile.TemporaryDirectory() as tmp:
        rc, objects, err = git(base, "rev-parse", "--path-format=absolute", "--git-path", "objects")
        if rc != 0:
            return None, err
        env = dict(os.environ, GIT_INDEX_FILE=os.path.join(tmp, "index"),
                   GIT_OBJECT_DIRECTORY=os.path.join(tmp, "objects"),
                   GIT_ALTERNATE_OBJECT_DIRECTORIES=objects)
        os.makedirs(env["GIT_OBJECT_DIRECTORY"])
        rc, _, err = git(base, "read-tree", commit, env=env)
        if rc != 0:
            return None, err
        for p in patches:
            rc, _, err = git(base, "apply", "--cached", "--whitespace=nowarn", p, env=env)
            if rc != 0:
                return None, f"{p} does not apply to {commit[:12]}: {err}"
        rc, out, err = git(base, "diff-index", "--cached", "--no-renames", "-z", "--name-status", commit, env=env, raw=True)
        if rc != 0:
            return None, err
        recs = [r for r in out.split("\0") if r]
        changed = {}
        for status, path in zip(recs[0::2], recs[1::2]):
            changed[path] = None if status.startswith("D") else ""
        kept = [p for p, v in changed.items() if v is not None]
        if kept:
            rc, out, err = git(base, "ls-files", "-s", "-z", "--", *kept, env=env, raw=True)
            if rc != 0:
                return None, err
            for rec in out.split("\0"):
                if rec:
                    meta, path = rec.split("\t", 1)
                    changed[path] = meta.split()[1]
        return changed, ""


def check_embargo(repo, m, supplied, ref, sref, sd):
    """The paths the recorded pre-disclosure patches change, after showing the tree holds exactly them.

    Refuses when the marker's patches are not the overlay commit's, a digest
    differs, a patch does not apply, or a patched file is not as the patches make it.
    """
    reassemble = "fix: re-assemble from the overlay checkout (its assemble.sh into a new directory) and deploy from that tree."
    entries = embargo_entries(m)
    carried = sorted(p for p in supplied if PATCH.match(p))
    if not entries and not carried:
        return set()
    listed = sorted(p for p, _ in entries)
    if listed != carried:
        refuse("GB-064", "the pre-disclosure patches the marker records are not the ones the overlay commit carries",
               ["marker: " + (", ".join(listed) or "(none)"), "overlay: " + (", ".join(carried) or "(none)"), reassemble])
    for p, digest in entries:
        f = os.path.join(repo, p)
        with open(f, "rb") as fh:
            got = hashlib.sha256(fh.read()).hexdigest()
        if got != digest:
            refuse("GB-064", f"{p} in this tree has sha256 {got[:12]}, the marker records {digest[:12]}", [reassemble])
    order = sorted(p for p, _ in entries)
    core = [os.path.join(repo, p) for p in order if p.endswith("/core.patch")]
    stuff = [os.path.join(repo, p) for p in order if p.endswith("/stuff.patch")]
    want = {}
    if core:
        got, err = patched(repo, ref, core)
        if got is None:
            refuse("GB-065", f"could not apply the pre-disclosure patches to goatcore {ref[:12]} ({err})", [reassemble])
        want.update(got)
    if stuff:
        if sref is None or not sd or not os.path.isdir(sd):
            refuse("GB-065", "the overlay carries goatstuff patches but this tree names no goatstuff checkout to apply them to", [reassemble])
        got, err = patched(sd, sref, stuff)
        if got is None:
            refuse("GB-065", f"could not apply the pre-disclosure patches to goatstuff {sref[:12]} ({err})", [reassemble])
        both = sorted(set(got) & set(want))
        if both:
            refuse("GB-064", "a path is changed by both a goatcore and a goatstuff patch: " + ", ".join(both[:5]), [reassemble])
        want.update(got)
    gone = sorted(p for p, b in want.items() if b is None and os.path.lexists(os.path.join(repo, p)))
    diff, err = differing(repo, {p: b for p, b in want.items() if b is not None})
    if diff is None:
        refuse("GB-065", f"could not read the patched files ({err})", [reassemble])
    bad = diff + gone
    if bad:
        shown = "\n    ".join(bad[:10]) + (f"\n    … and {len(bad) - 10} more" if len(bad) > 10 else "")
        refuse("GB-064", f"{len(bad)} file(s) the pre-disclosure patches change are not in this tree as the pinned commits plus the patches make them",
               ["changed or missing:\n    " + shown, reassemble])
    return set(want)


def overlay_tree(od, commit):
    rc, out, _ = git(od, "ls-tree", "-r", "-z", "--name-only", "--full-tree", commit)
    return set(p for p in out.split("\0") if p) if rc == 0 else set()


def check(repo, offline, quiet):
    m = marker(repo)
    if m is None:
        say(f"{repo} has no {MARKER}: not an assembled tree"); sys.exit(2)
    ref, oc = m.get("goatcore_ref", ""), m.get("overlay_commit", "")
    reassemble = "fix: re-assemble from the overlay checkout (its assemble.sh into a new directory) and deploy from that tree."
    if not SHA.match(ref) or not oc:
        refuse("GB-064", f"{MARKER} must carry goatcore_ref=<40-hex commit> and overlay_commit=<commit>", [reassemble])
    # goatcore.ref in the tree is the overlay's declaration; the marker says what was cloned.
    decl = read_ref(repo, "goatcore.ref")
    if decl is not None and decl != ref:
        refuse("GB-064", f"goatcore.ref in this tree names {decl[:12]} but the tree was assembled on goatcore {ref[:12]}", [reassemble])
    sref = m.get("goatstuff_ref")
    if sref is not None:
        if not SHA.match(sref):
            refuse("GB-064", f"{MARKER} carries goatstuff_ref= but not a 40-hex commit", [reassemble])
        sdecl = read_ref(repo, "goatstuff.ref")
        if sdecl is not None and sdecl != sref:
            refuse("GB-064", f"goatstuff.ref in this tree names {sdecl[:12]} but the tree was assembled with goatstuff {sref[:12]}", [reassemble])
    rc, head, _ = git(repo, "rev-parse", "HEAD")
    if rc != 0 or head != ref:
        refuse("GB-064", f"this tree's goatcore checkout is at {head[:12] or '(no commit)'}, not the goatcore.ref commit {ref[:12]}", [reassemble])
    pinned = os.environ.get("GOAT_FRESH_SOURCE_TRUNK", "")
    if pinned and pinned != ref:
        say(f"GOAT_FRESH_SOURCE_TRUNK={pinned} is not this tree's goatcore commit {ref}"); sys.exit(2)
    if oc.endswith("-dirty"):
        refuse("GB-064", f"the overlay was assembled from uncommitted changes ({oc[:12]}-dirty), so no commit records what this tree deploys",
               ["fix: commit the overlay change, then re-assemble."])
    od = overlay_dir(m)
    if not od or not os.path.isdir(od):
        if offline:
            say(f"OFFLINE — no overlay checkout to judge overlay {oc[:12]} against; proceeding as an offline run.")
            print(ref); return
        refuse("GB-065", "the overlay checkout this tree was assembled from is not known, so the tree cannot be shown current with the overlay",
               ["fix: set GOAT_OVERLAY_DIR=<overlay checkout> (or write overlay_dir= into .goat-assembled), and re-run.",
                "On an islanded site, re-run with --offline."])
    rc, ocsha, _ = git(od, "rev-parse", "--verify", "--quiet", oc + "^{commit}")
    if rc != 0:
        refuse("GB-064", f"overlay commit {oc[:12]} is not in the overlay checkout {od}", [reassemble])
    diff, err = tree_matches_overlay(repo, od, ocsha)
    if diff is None:
        refuse("GB-065", f"could not read overlay commit {oc[:12]} ({err})", [reassemble])
    if diff:
        shown = "\n    ".join(diff[:10]) + (f"\n    … and {len(diff) - 10} more" if len(diff) > 10 else "")
        refuse("GB-064", f"{len(diff)} file(s) this tree deploys from differ from overlay commit {oc[:12]} (edited after assembly)",
               ["changed or missing:\n    " + shown,
                "Edit the overlay, commit, re-assemble; a deploy renders only what a commit records.", reassemble])
    supplied = overlay_tree(od, ocsha)
    sd = goatstuff_dir(m) if sref is not None else ""
    if sref is not None:
        if not sd or not os.path.isdir(sd):
            if offline:
                say(f"OFFLINE — no goatstuff checkout to judge goatstuff {sref[:12]} against; proceeding as an offline run.")
                print(ref); return
            refuse("GB-065", f"the goatstuff checkout this tree was assembled from is not known, so the tree cannot be shown to hold goatstuff {sref[:12]}",
                   ["fix: set GOAT_GOATSTUFF_DIR=<goatstuff checkout> (or write goatstuff_path= into .goat-assembled), and re-run.",
                    "On an islanded site, re-run with --offline."])
        rc, _, _ = git(sd, "rev-parse", "--verify", "--quiet", sref + "^{commit}")
        if rc != 0:
            refuse("GB-064", f"goatstuff commit {sref[:12]} is not in the goatstuff checkout {sd}", [reassemble])
    embargoed = check_embargo(repo, m, supplied, ref, sref, sd)
    if sref is not None:
        sdiff, err = tree_matches_goatstuff(repo, sd, sref, supplied, embargoed)
        if sdiff is None:
            refuse("GB-065", f"could not read goatstuff commit {sref[:12]} ({err})", [reassemble])
        if sdiff:
            shown = "\n    ".join(sdiff[:10]) + (f"\n    … and {len(sdiff) - 10} more" if len(sdiff) > 10 else "")
            refuse("GB-064", f"{len(sdiff)} goatstuff file(s) this tree deploys from differ from goatstuff {sref[:12]} (edited after assembly, or assembled from another commit)",
                   ["changed or missing:\n    " + shown,
                    "A change to goatstuff lands in goatstuff and reaches the tree by moving goatstuff.ref.", reassemble])
        sw, _ = blobs(sd, sref, ["ops"])
        supplied = supplied | set(sw or {})
    stray, err = stray_core_edits(repo, supplied | embargoed)
    if stray is None:
        refuse("GB-065", f"could not read this tree's git status ({err})", [reassemble])
    if stray:
        shown = "\n    ".join(stray[:10]) + (f"\n    … and {len(stray) - 10} more" if len(stray) > 10 else "")
        refuse("GB-064", f"{len(stray)} goatcore file(s) this tree deploys from were changed in the tree and come from neither goatcore {ref[:12]}" + (" nor goatstuff" if sref is not None else "") + " nor the overlay",
               ["changed:\n    " + shown,
                "A change to goatcore lands in goatcore and reaches the tree by moving goatcore.ref; an environment's change lands in the overlay.", reassemble])
    if pinned:
        print(ref); return  # judged once by the driver; no fetch, same commits for every play
    remote = os.environ.get("GOAT_OVERLAY_TRUNK_REMOTE") or os.environ.get("GOAT_OVERLAY_REMOTE") or "origin"
    trunk = f"{remote}/main"
    if offline:
        rc, tsha, _ = git(od, "rev-parse", "--verify", "--quiet", trunk + "^{commit}")
        if rc != 0:
            say(f"OFFLINE — no cached {trunk} in the overlay checkout; proceeding as an offline run.")
            print(ref); return
        say(f"OFFLINE — not fetching the overlay; judging overlay {oc[:12]} against the cached {trunk} at {tsha[:12]}. The run is recorded as offline.")
    else:
        env = dict(os.environ)
        env.setdefault("GIT_SSH_COMMAND", "ssh -o BatchMode=yes -o ConnectTimeout=10")
        rc, _, _ = git(od, "fetch", "--quiet", remote, "main", env=env)
        if rc != 0:
            refuse("GB-065", f"could not fetch {remote} main in the overlay checkout {od}, so overlay {oc[:12]} cannot be shown current",
                   [f"fix: restore the connection to the overlay's {remote} and re-run.",
                    f"On an islanded site, re-run with --offline: the run is then judged against the cached {trunk} and recorded as offline."])
        rc, tsha, _ = git(od, "rev-parse", "--verify", "--quiet", trunk + "^{commit}")
        if rc != 0:
            refuse("GB-065", f"fetched {remote}, but the overlay has no main branch to judge against", [])
    rc, behind, _ = git(od, "log", "--format=%h %s", f"{ocsha}..{tsha}", "--", ".", *NOT_DEPLOYED)
    if behind:
        lines = behind.splitlines()
        shown = "\n    ".join(lines[:10]) + (f"\n    … and {len(lines) - 10} more" if len(lines) > 10 else "")
        refuse("GB-064", f"this tree's overlay {oc[:12]} is behind the overlay's {trunk} in what a deploy reads (everything but docs/)",
               [f"overlay {oc[:12]}; {trunk} {tsha[:12]}",
                f"{len(lines)} commit(s) on {trunk} not in this tree change those paths (newest first):\n    " + shown,
                "A role, play or group_vars file rendered from this tree would remove on the host what those commits added, and the play would report success.",
                f"fix: git -C {od} pull --ff-only {remote} main, then re-assemble and deploy from the new tree."])
    if not quiet:
        rc, ahead, _ = git(od, "log", "--format=%h %s", f"{tsha}..{ocsha}")
        if ahead:
            lines = ahead.splitlines()
            say(f"WARN — this tree's overlay carries {len(lines)} commit(s) not on {trunk}; they are deployed with this run:")
            for line in lines[:5]:
                print(f"    {line}", file=sys.stderr)
        exp = export_of(repo)
        ids = embargo_ids(embargo_entries(m))
        say(f"assembled tree: goatcore {ref[:12]}" + (f" (export of {exp})" if exp else "")
            + (f" + goatstuff {sref[:12]}" if sref is not None else "") + f" + overlay {oc[:12]}"
            + (f" + pre-disclosure patches {', '.join(ids)}" if ids else ""))
    print(ref)


def describe(repo):
    m = marker(repo)
    if m is None:
        rc, head, _ = git(repo, "rev-parse", "HEAD")
        return {"commit": head} if rc == 0 else {}
    d = {"goatcore": m.get("goatcore_ref", ""), "overlay": m.get("overlay_commit", "")}
    decl = read_ref(repo, "goatcore.ref")
    if decl is not None:
        d["goatcore_ref"] = decl
    if "goatstuff_ref" in m:
        d["goatstuff"] = m["goatstuff_ref"]
        sdecl = read_ref(repo, "goatstuff.ref")
        if sdecl is not None:
            d["goatstuff_ref"] = sdecl
    exp = export_of(repo)
    if exp:
        d["goatcore_export_of"] = exp
    ids = embargo_ids(embargo_entries(m))
    if ids:
        d["embargo"] = ids
    return d


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("cmd", choices=["kind", "check", "describe"])
    ap.add_argument("--repo", required=True)
    ap.add_argument("--offline", action="store_true")
    ap.add_argument("--quiet", action="store_true")
    a = ap.parse_args()
    repo = os.path.abspath(a.repo)
    if a.cmd == "kind":
        print("assembled" if marker(repo) is not None else "clone")
    elif a.cmd == "check":
        check(repo, a.offline or os.environ.get("GOAT_DEPLOY_OFFLINE") == "1", a.quiet)
    else:
        print(json.dumps(describe(repo), sort_keys=True, separators=(",", ":")))


if __name__ == "__main__":
    main()
