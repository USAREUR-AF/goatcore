# SPDX-License-Identifier: Apache-2.0
"""
Merge one writer's keys into a shared dotenv file without dropping anyone else's.

/opt/mgmt-host/.env has many writers. mgmt-host.yml renders the keys it owns;
other roles add theirs with lineinfile (image pins, the Kratos courier URI, the
SPIRE join token, dashboard settings and more). A writer that rewrites the whole
file deletes those keys on its next run, and `docker compose up` then recreates
the affected services on their compose defaults or refuses to start them.

    {{ existing_text | dotenv_merge(rendered_text) }}
    {{ existing_text | dotenv_key_report(rendered_text) }}

dotenv_merge keeps the existing file's lines and order. Each key the render
carries replaces that key's first line in place (later duplicates of it are
dropped); keys the render adds go at the end. Comment lines in the render are
not copied, so a timestamped header never changes the file, and a re-run with
the same render is a no-op.

dotenv_key_report returns key NAMES only, never values: added, changed,
unchanged (keys this writer owns) and kept (keys it leaves to other writers).
"""

import re

_KEY = re.compile(r"^([A-Za-z_][A-Za-z0-9_]*)=(.*)$")


def _pairs(text):
    out = {}
    for line in (text or "").splitlines():
        m = _KEY.match(line)
        if m and m.group(1) not in out:
            out[m.group(1)] = m.group(2)
    return out


def _rendered(text):
    order, values = [], {}
    for line in (text or "").splitlines():
        m = _KEY.match(line)
        if not m:
            continue
        if m.group(1) not in values:
            order.append(m.group(1))
        values[m.group(1)] = m.group(2)
    return order, values


def dotenv_merge(existing, rendered):
    order, values = _rendered(rendered)
    out, seen = [], set()
    for line in (existing or "").splitlines():
        m = _KEY.match(line)
        if m and m.group(1) in values:
            k = m.group(1)
            if k in seen:
                continue
            seen.add(k)
            out.append(f"{k}={values[k]}")
        else:
            out.append(line)
    if not existing:
        out.append("# Keys are written by several roles; mgmt-host.yml updates only its own.")
    for k in order:
        if k not in seen:
            out.append(f"{k}={values[k]}")
    return "\n".join(out) + "\n"


def dotenv_key_report(existing, rendered):
    old = _pairs(existing)
    order, values = _rendered(rendered)
    added = sorted(k for k in order if k not in old)
    changed = sorted(k for k in order if k in old and old[k] != values[k])
    unchanged = sorted(k for k in order if k in old and old[k] == values[k])
    kept = sorted(k for k in old if k not in values)
    return {"added": added, "changed": changed, "unchanged": unchanged, "kept": kept}


class FilterModule:
    def filters(self):
        return {"dotenv_merge": dotenv_merge, "dotenv_key_report": dotenv_key_report}
