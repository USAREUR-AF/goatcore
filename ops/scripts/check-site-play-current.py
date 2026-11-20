#!/usr/bin/env python3
"""check-site-play-current.py: a site's own copy of a generic play is current.

The drivers run a site's own `ops/ansible/<site>-<suffix>` in place of the
generic `ops/ansible/site-<suffix>` whenever the deployment keeps one
(crates/goatbuilder/src/standup.rs site_play). A copy made from an older generic
play silently misses every fix the generic play gained since. This check, run
right before the drivers use the site's play:

- passes when the site keeps no copy: the generic play runs;
- passes a wrapper that imports the generic play (`import_playbook:
  site-<suffix>`), which cannot go stale;
- passes a copy that carries `# derived-from: site-<suffix> sha256:<hex>` with
  the current generic play's digest;
- refuses (GB-362, exit 1) a copy with no such line, or with another digest.

usage: check-site-play-current.py [--repo DIR] <site> <suffix>
exit:  0 current or no copy, 1 refused, 2 usage
"""
import argparse
import hashlib
import re
import sys
from pathlib import Path

import yaml

LINE = re.compile(r"^#\s*derived-from:\s*(\S+)\s+sha256:([0-9a-f]{64})\s*$")


def imports_generic(text, generic):
    try:
        doc = yaml.safe_load(text)
    except yaml.YAMLError:
        return False
    plays = doc if isinstance(doc, list) else []
    return any(isinstance(p, dict) and generic in (p.get("ansible.builtin.import_playbook"), p.get("import_playbook"))
               for p in plays)


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--repo", default=".", help="the checkout (default: the current directory)")
    ap.add_argument("site")
    ap.add_argument("suffix", help="the play's suffix, e.g. seed-from-peer.yml")
    a = ap.parse_args()
    plays = Path(a.repo) / "ops/ansible"
    generic, own = f"site-{a.suffix}", f"{a.site}-{a.suffix}"
    if not (plays / own).is_file():
        print(f"check-site-play-current: {a.site} keeps no site copy of {generic}; the generic play runs")
        return 0
    if not (plays / generic).is_file():
        print(f"GB-362 site play: ops/ansible/{generic} is not in this checkout, so {own} cannot be checked against it", file=sys.stderr)
        return 1
    text = (plays / own).read_text()
    if imports_generic(text, generic):
        print(f"check-site-play-current: {own} imports {generic}; it runs the current generic play")
        return 0
    want = hashlib.sha256((plays / generic).read_bytes()).hexdigest()
    lines = [m for m in (LINE.match(l) for l in text.splitlines()) if m and m.group(1) == generic]
    if not lines:
        print(f"GB-362 site play: ops/ansible/{own} is a copy of {generic} that does not say which version it was made from, "
              f"so fixes made to the generic play since cannot be known to be in it. Compare it with the current {generic}, "
              f"bring it up to date, and add this line at its top:\n  # derived-from: {generic} sha256:{want}", file=sys.stderr)
        return 1
    have = lines[0].group(2)
    if have != want:
        print(f"GB-362 site play: ops/ansible/{own} was derived from {generic} at sha256:{have}; the current {generic} is "
              f"sha256:{want}. The generic play changed since the copy was made. Compare the two, carry its changes into "
              f"{own}, and set its derived-from line to sha256:{want}", file=sys.stderr)
        return 1
    print(f"check-site-play-current: {own} was derived from the current {generic} (sha256:{want[:12]})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
