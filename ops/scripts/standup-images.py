#!/usr/bin/env python3
"""standup-images.py — every pinned image one member host-group deploys, one per line.

The list a standup pre-pulls on the box after phase 2 (ops/ansible/standup-image-pull.yml,
docs/design/goatlite.md §11 measure 2). It is read from the same merged pin table the
deploy reads (ops/pins/fleet-pins.yaml plus the lite sidecar ops/pins/lite-pins.yaml,
through goat_consistency.resolve), so it holds exactly the image pins rendered into
group_vars/<member>/pins.yml: components declared `absent` for the member are left out,
non-image artifacts are left out, duplicates are printed once.

  standup-images.py --member <group> [--ghcr-org <org>] [--table <fleet-pins.yaml>]
  standup-images.py --preset lite [--image-registry <host:port/ns>] [--ghcr-org <org>]

`--preset lite` lists what ANY goatlite would run before one is rendered: the fleet
table with the deviations ops/scripts/goatlite-render.py writes for a goatlite (the
same function, lite_deviations). The prepared VM image (ops/vm-image/) and the
local-registry recipe (ops/recipes/local-registry/) read this list.

`{{ ghcr_org }}` in a pin is replaced by --ghcr-org (default: $GHCR_ORG, else the default
ops/ansible/group_vars/all.yml sets for ghcr_org). A pin carrying any other template is
refused by name, and so is a `{{ ghcr_org }}` pin when no organisation is known.
A member the table does not govern prints nothing and says so on stderr: such a goatnet
pulls its images inside its plays, as before.

Exit: 0 listed (possibly empty); 2 a pin this script cannot resolve, or a bad table.
"""
import argparse
import importlib.util
import os
import re
import sys

REPO = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))
sys.path.insert(0, os.path.join(REPO, "ops", "consistency", "python"))

try:
    import yaml  # noqa: E402
    from goat_consistency import resolve  # noqa: E402
except ImportError as exc:  # pragma: no cover - dependency, not logic
    sys.exit("standup-images: %s (PyYAML + jsonschema required: pip install pyyaml jsonschema)" % exc)


def default_ghcr_org(repo=REPO):
    """$GHCR_ORG, else the ghcr_org default group_vars/all.yml sets: the namespace
    every play of this checkout renders a pin with. Empty when neither names one."""
    org = os.environ.get("GHCR_ORG") or ""
    if org:
        return org
    try:
        with open(os.path.join(repo, "ops", "ansible", "group_vars", "all.yml"), encoding="utf-8") as fh:
            m = re.search(r"""^ghcr_org:.*["' ]([A-Za-z0-9][A-Za-z0-9._-]*)["']?[ }"]*$""", fh.read(), re.M)
    except OSError:
        return ""
    return m.group(1) if m else ""


def _resolved(cname, value, ghcr_org, out, bad):
    ref = str(value).replace("{{ ghcr_org }}", ghcr_org) if ghcr_org else str(value)
    if "{{" in ref or "{%" in ref:
        bad.append("%s: %s" % (cname, value))
    elif ref not in out:
        out.append(ref)


def images(member, ghcr_org, table=None):
    t = resolve.load_tables(table)
    member_of = resolve.validate(t.goatnets, t.components)
    goatnet = member_of.get(member)
    if goatnet is None:
        return None
    out, bad = [], []
    for cname, spec in t.components.items():
        if (spec.get("artifact_type") or "image") != "image":
            continue
        value, _dev = resolve.resolve(spec, member, goatnet)
        if value is None:
            continue  # declared absent for this member, or no fleet value
        _resolved(cname, value, ghcr_org, out, bad)
    if bad:
        raise ValueError("a pin carries a template this script does not resolve: " + "; ".join(bad))
    return out


def renderer():
    spec = importlib.util.spec_from_file_location("goatlite_render", os.path.join(REPO, "ops/scripts/goatlite-render.py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def lite_images(ghcr_org, image_registry=""):
    """Every image a goatlite runs: the fleet value, or the deviation the renderer writes."""
    r = renderer()
    devs = r.lite_deviations(image_registry)
    # The components alone: the preset lists what a goatlite would run before one
    # is declared, so a checkout that declares no goatnet (goatcore) still lists them.
    with open(r.FLEET_PINS, encoding="utf-8") as fh:
        components = (yaml.safe_load(fh) or {}).get("components") or {}
    if not isinstance(components, dict) or not components:
        raise ValueError("%s: `components:` must be a non-empty mapping" % r.FLEET_PINS)
    out, bad = [], []
    for cname, spec in components.items():
        if (spec.get("artifact_type") or "image") != "image":
            continue
        dev = devs.get(cname)
        if dev is not None and dev[0] == "absent":
            continue
        value = dev[1] if dev is not None else spec.get("fleet")
        if value is None:
            continue
        _resolved(cname, value, ghcr_org, out, bad)
    if bad:
        raise ValueError("a pin carries a template this script does not resolve: " + "; ".join(bad))
    return out


def main():
    ap = argparse.ArgumentParser(description="Every pinned image a member host-group deploys, one per line.")
    who = ap.add_mutually_exclusive_group(required=True)
    who.add_argument("--member", help="the host-group, e.g. pasture_hosts")
    who.add_argument("--preset", choices=["lite"], help="every image any goatlite runs, before one is rendered")
    ap.add_argument("--ghcr-org", default=default_ghcr_org(),
                    help="the ghcr.io organisation a pin's {{ ghcr_org }} stands for "
                         "(default: $GHCR_ORG, else ghcr_org in ops/ansible/group_vars/all.yml)")
    ap.add_argument("--image-registry", default="", help="with --preset lite: the lite.image_registry to list against")
    ap.add_argument("--table", default=None, help="the fleet table to read (default: ops/pins/fleet-pins.yaml)")
    a = ap.parse_args()
    try:
        if a.preset:
            refs = lite_images(a.ghcr_org, a.image_registry.strip().rstrip("/"))
        else:
            refs = images(a.member, a.ghcr_org, a.table)
    except (ValueError, resolve.SchemaError) as e:
        print("standup-images: %s" % e, file=sys.stderr)
        return 2
    if refs is None:
        print("standup-images: %s is governed by no pin table; nothing to pre-pull" % a.member, file=sys.stderr)
        return 0
    for r in refs:
        print(r)
    return 0


if __name__ == "__main__":
    sys.exit(main())
