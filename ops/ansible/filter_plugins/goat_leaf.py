"""
Where a site's leaf certificate or key lives on the controller (#2165).

mint-site-certs.sh writes each leaf to the site manifest's `out_dir`
(<state root>/<site>-leaves by convention), outside every checkout. The state
root is the play variable goat_controller_state_dir; a caller outside a play
gets the value group_vars/all.yml declares for it; with neither, ~/.goat/state.
Plays used to read a gitignored copy staged inside the checkout
(ops/enrollment/<site>/), so a fresh worktree, which every deploy now runs
from (C3, #2124), had no leaf until the staging step was re-run.

    {{ 'router-cert.pem' | goat_leaf(site_id) }}
    {{ 'zagros/names-collector-cert.pem' | goat_leaf }}     # site/file form

The first existing file wins:
  1. $GOAT_LEAF_DIR/<file>                      an explicit override
  2. <manifest out_dir>/<file>                  the per-operator state dir
  3. <this checkout>/ops/enrollment/<site>/<file>     the old staged copy
  4. <main checkout>/ops/enrollment/<site>/<file>     the same, seen from a worktree
When none exists the state-dir path (2) is returned, so a pre-flight that
stats it names the place the leaf belongs. Keys are never read here: only
paths are computed.
"""

import os
import re
import subprocess
from pathlib import Path

try:
    from jinja2 import pass_context
except ImportError:  # a script that imports this module without Ansible calls goat_leaf() itself
    def pass_context(f):
        return f

OPS = Path(__file__).resolve().parents[2]
REPO = OPS.parent
STATE_ROOT_VAR = "goat_controller_state_dir"
STATE_ROOT_DEFAULT = "~/.goat/state"
_HOME_LOOKUP = re.compile(r"\{\{\s*lookup\(\s*'env'\s*,\s*'HOME'\s*\)\s*\}\}")


def _split(name, site):
    if site:
        return site, name
    if "/" not in name:
        raise ValueError(f"goat_leaf: '{name}' needs a site: '{name}' | goat_leaf(site_id)")
    site, _, rest = name.partition("/")
    return site, rest


def declared_state_root():
    """goat_controller_state_dir as group_vars/all.yml declares it, for a caller outside a play."""
    try:
        lines = (OPS / "ansible" / "group_vars" / "all.yml").read_text(encoding="utf-8").splitlines()
    except OSError:
        return None
    for line in lines:
        if line.startswith(STATE_ROOT_VAR + ":"):
            value = line.split(":", 1)[1].strip().strip("'\"")
            value = _HOME_LOOKUP.sub("~", value)
            return None if "{{" in value else value
    return None


def state_dir(site, root=None):
    """The manifest's out_dir, else <state root>/<site>-leaves."""
    manifest = OPS / "enrollment" / "manifests" / f"{site}-certs.yaml"
    out = None
    if manifest.is_file():
        for line in manifest.read_text(encoding="utf-8").splitlines():
            if line.startswith("out_dir:"):
                out = line.split(":", 1)[1].split("#", 1)[0].strip().strip("'\"")
                break
    if not out:
        out = f"{root or declared_state_root() or STATE_ROOT_DEFAULT}/{site}-leaves"
    return Path(os.path.expanduser(out))


def _main_checkout():
    try:
        common = subprocess.run(
            ["git", "-C", str(REPO), "rev-parse", "--path-format=absolute", "--git-common-dir"],
            capture_output=True, text=True, timeout=5, check=True,
        ).stdout.strip()
    except (OSError, subprocess.SubprocessError):
        return None
    main = Path(common).parent
    return None if main == REPO else main


def candidates(name, site=None, root=None):
    site, rel = _split(name, site)
    out = []
    override = os.environ.get("GOAT_LEAF_DIR")
    if override:
        out.append(Path(os.path.expanduser(override)) / rel)
    out.append(state_dir(site, root) / rel)
    out.append(OPS / "enrollment" / site / rel)
    main = _main_checkout()
    if main:
        out.append(main / "ops" / "enrollment" / site / rel)
    return out


def goat_leaf(name, site=None, root=None):
    paths = candidates(name, site, root)
    for p in paths:
        if p.is_file():
            return str(p)
    return str(paths[1] if os.environ.get("GOAT_LEAF_DIR") else paths[0])


def goat_leaf_mint(name, site=None):
    """The command that puts a missing leaf where goat_leaf looks first."""
    site, rel = _split(name, site)
    leaf = rel.rsplit("-", 1)[0] if rel.endswith(("-cert.pem", "-key.pem")) else rel
    return f"ops/scripts/mint-site-certs.sh {site} --only {leaf}"


@pass_context
def _goat_leaf_filter(context, name, site=None):
    """goat_leaf inside a play: the state root is the play's goat_controller_state_dir."""
    root = context.resolve_or_missing(STATE_ROOT_VAR)
    return goat_leaf(name, site, str(root) if isinstance(root, str) and root else None)


class FilterModule:
    def filters(self):
        return {"goat_leaf": _goat_leaf_filter, "goat_leaf_mint": goat_leaf_mint}
