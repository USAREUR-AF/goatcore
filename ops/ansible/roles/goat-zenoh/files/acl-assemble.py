#!/usr/bin/env python3
"""acl-assemble — reconcile the zenoh router ACL from a role-owned SEED and a
portal-owned ORG fragment.

The router loads ONE config.json5 (zenohd takes a single --config; its config
merge REPLACES arrays, so a second file can't append rules). So instead of the
portal editing the live config in place — which a role re-render then clobbers —
the ACL has two owners and an assembler:

  SEED   (role)   : config.json5 as the template renders it, with the four
                    per-org marker regions EMPTY (storages / write rules /
                    subjects / policies).
  ORG    (portal) : just the per-org blocks add-vendor/add-org appends, one
                    labeled section per region.
  LIVE            : assemble(SEED, ORG) — the org blocks spliced back between
                    each region's BEGIN/END markers. This is what zenohd loads.

The role writes the seed and runs `assemble`; the portal appends to the org
fragment and runs `assemble`. A role re-render regenerates only the seed, so it
can no longer drop org rules — the split makes the clobber impossible.

  extract  LIVE  -> SEED + ORG   (one-time migration from a live in-place config)
  assemble SEED + ORG -> LIVE    (steady state; run by the role and the portal)

Round-trip is byte-exact: assemble(extract(x)) == x.

Usage:
  acl-assemble.py extract  --live L --seed S --org O
  acl-assemble.py assemble --seed S --org O --out L
  acl-assemble.py init-org --org O          # write an empty fragment (idempotent)

Ownership split (the "org = portal-only" model, F-258 Phase-1 wiring):
  The role STILL renders the seed vendors (zenoh_seed_vendors) inline in the
  seed — the template is unchanged. The org fragment carries ONLY the vendors the
  portal/helper adds at runtime. So on a FRESH standup the org fragment is empty
  and assemble(seed, empty) == seed byte-for-byte (splicing empty content at each
  END marker is a no-op) — every existing goatnet renders exactly as before. Only
  an already-live router gets a one-time migration that moves its runtime vendors
  into the fragment.
"""
import argparse
import sys

# region key -> (BEGIN marker substring, END marker substring). The BEGIN line
# may carry trailing prose; match by substring. Order is display-only.
REGIONS = [
    ("storages",    "// BEGIN: per-vendor storages",    "// END: per-vendor storages"),
    ("write-rules", "// BEGIN: per-vendor write rules",  "// END: per-vendor write rules"),
    ("subjects",    "// BEGIN: per-vendor subjects",     "// END: per-vendor subjects"),
    ("policies",    "// BEGIN: per-vendor policies",     "// END: per-vendor policies"),
]
FRAG_HDR = "##ACL-ORG-REGION "  # fragment section header; never appears in ACL content


def _line_indices(lines, begin_sub, end_sub):
    b = e = None
    for i, ln in enumerate(lines):
        if b is None and begin_sub in ln:
            b = i
        elif b is not None and end_sub in ln:
            e = i
            break
    if b is None or e is None or e <= b:
        sys.exit(f"acl-assemble: markers not found or malformed for '{end_sub}'")
    return b, e


def extract(live_path, seed_path, org_path):
    with open(live_path, encoding="utf-8") as source_file:
        lines = source_file.readlines()  # keeps line endings
    org_sections = []
    # collect (begin_idx, end_idx, content_lines) for every region, then rebuild
    spans = []
    for key, begin_sub, end_sub in REGIONS:
        b, e = _line_indices(lines, begin_sub, end_sub)
        content = lines[b + 1:e]           # strictly between the markers
        org_sections.append((key, content))
        spans.append((b + 1, e))
    # SEED = live with every region's content removed (markers left adjacent)
    seed, prev = [], 0
    for (start, end) in spans:
        seed.extend(lines[prev:start])
        prev = end
    seed.extend(lines[prev:])
    with open(seed_path, "w", encoding="utf-8") as source_file:
        source_file.writelines(seed)
    # ORG = each region's content under a header
    with open(org_path, "w", encoding="utf-8") as f:
        for key, content in org_sections:
            f.write(f"{FRAG_HDR}{key}\n")
            f.writelines(content)
    rules = sum(1 for _k, c in org_sections for ln in c if "permission:" in ln)
    print(f"acl-assemble extract: {rules} org write-rules across "
          f"{len([1 for _k, c in org_sections if c])} non-empty regions")


def init_org(org_path):
    """Write an empty org fragment: just the four region headers, no content.
    Idempotent — refuses to clobber a fragment that already has org content, so a
    re-run of the role can never blow away portal-added vendors. A header-only
    fragment (or a missing file) is (re)written; anything with body lines is left
    untouched."""
    import os
    if os.path.exists(org_path):
        with open(org_path, encoding="utf-8") as source_file:
            has_content = any(
                ln.strip() and not ln.startswith(FRAG_HDR)
                for ln in source_file.readlines()
            )
        if has_content:
            print(f"acl-assemble init-org: {org_path} already has org content; left intact")
            return
    with open(org_path, "w", encoding="utf-8") as f:
        for key, _b, _e in REGIONS:
            f.write(f"{FRAG_HDR}{key}\n")
    print(f"acl-assemble init-org: wrote empty fragment {org_path}")


def _load_org(org_path):
    sections, cur = {k: [] for k, _b, _e in REGIONS}, None
    with open(org_path, encoding="utf-8") as source_file:
        for ln in source_file.readlines():
            if ln.startswith(FRAG_HDR):
                cur = ln[len(FRAG_HDR):].strip()
                if cur not in sections:
                    sys.exit(f"acl-assemble: unknown org region '{cur}'")
            elif cur is not None:
                sections[cur].append(ln)
    return sections


def assemble(seed_path, org_path, out_path):
    with open(seed_path, encoding="utf-8") as source_file:
        lines = source_file.readlines()
    org = _load_org(org_path)
    out = []
    # walk the seed; when we hit a region's END line, emit that region's org
    # content immediately before it (matching add-vendor.sh's insert point)
    end_map = {end_sub: key for key, _b, end_sub in REGIONS}
    for ln in lines:
        matched = next((es for es in end_map if es in ln), None)
        if matched is not None:
            out.extend(org[end_map[matched]])
        out.append(ln)
    with open(out_path, "w", encoding="utf-8") as source_file:
        source_file.writelines(out)
    rules = sum(1 for c in org.values() for ln in c if "permission:" in ln)
    print(f"acl-assemble assemble: spliced {rules} org write-rules into {out_path}")


def main():
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)
    ex = sub.add_parser("extract")
    ex.add_argument("--live", required=True)
    ex.add_argument("--seed", required=True)
    ex.add_argument("--org", required=True)
    asm = sub.add_parser("assemble")
    asm.add_argument("--seed", required=True)
    asm.add_argument("--org", required=True)
    asm.add_argument("--out", required=True)
    ini = sub.add_parser("init-org")
    ini.add_argument("--org", required=True)
    a = ap.parse_args()
    if a.cmd == "extract":
        extract(a.live, a.seed, a.org)
    elif a.cmd == "init-org":
        init_org(a.org)
    else:
        assemble(a.seed, a.org, a.out)


if __name__ == "__main__":
    main()
