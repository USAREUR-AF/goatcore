#!/usr/bin/env python3
"""Render the isitup row templates without ansible, for the role's tests.

The role builds ONE row list (templates/isitup-rows.json.j2 -> the
`isitup_rows` fact) and renders services.env and data/meta.json from it. This
does the same three steps with plain jinja2, stubbing the ansible-only names
the templates use: the `lookup` global (varnames + vars, for the ADR 1178
external-service declarations), `now`, and the `bool` / `to_json` /
`to_nice_json` filters. trim_blocks matches what ansible's template module
sets, because the rendered file's line structure is part of what is under test.

As a script (free-text-columns.sh): renders the fixture below, writes the
services.env to argv[1] and prints the expected rows as JSON on stdout:
{"rows": [{"id", "name", "desc"}]}.

As a module (row-list.py): `render(context, today)` returns
(rows, services_env_text, meta_dict). `rows_sha256(rows)` is the digest
tasks/rows-build.yml stamps the list with; list-freshness.sh holds the two
against each other.
"""
import datetime
import hashlib
import json
import os
import re
import sys

import jinja2

BS = chr(92)  # a literal backslash, spelled so no layer in between can halve it

# Every character bash acts on inside double quotes, in both free-text columns.
NASTY_NAME = 'Board "A" costs $5'
NASTY_DESC = (
    'Press "Live collaboration", share $HOME and `whoami` and one '
    + BS + ' backslash; "quoted words" again.'
)
# A description with embedded double quotes, the shape that has to round-trip.
EXCALIDRAW_DESC = (
    'Collaborative drawing and diagramming with live rooms: press "Live '
    'collaboration", share the link, everyone with it draws together. Rooms '
    'are end-to-end encrypted; the relay sees only ciphertext.'
)

CONTEXT = {
    "site_id": "test-site",
    "site_dns_zone": "example.invalid",
    "isitup_services": [
        {"id": "nasty", "name": NASTY_NAME, "desc": NASTY_DESC,
         "fqdn": "nasty.test-site.example.invalid", "type": "external"},
        {"id": "plain", "name": "Plain", "desc": "Nothing special here.",
         "fqdn": "plain.test-site.example.invalid", "type": "external"},
    ],
    "external_service_quoted": {
        "name": "quoted",
        "enabled": True,
        "status": {"name": 'Whiteboard ("Excalidraw")', "desc": EXCALIDRAW_DESC},
    },
}

EXPECTED = [
    {"id": "nasty", "name": NASTY_NAME, "desc": NASTY_DESC},
    {"id": "plain", "name": "Plain", "desc": "Nothing special here."},
    {"id": "quoted", "name": 'Whiteboard ("Excalidraw")', "desc": EXCALIDRAW_DESC},
]


def to_bool(value):
    if isinstance(value, str):
        return value.strip().lower() in ("1", "true", "yes", "on", "y", "t")
    return bool(value)


def rows_sha256(rows):
    """The digest tasks/rows-build.yml computes: sha256 of the row list as
    canonical JSON (keys sorted, no spaces, ASCII escapes)."""
    text = json.dumps(rows, sort_keys=True, separators=(",", ":"), ensure_ascii=True)
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


# What tasks/rows-build.yml and rows-render.yml hand meta.json.j2 besides the rows.
SOURCE = {"goatcore": "1" * 40, "overlay": "2" * 40}
RENDERED_AT = "2026-01-01T00:00:00Z"


def render(context, today=None):
    """Render rows -> services.env + meta.json exactly as tasks/main.yml does."""
    today = today or datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%d")
    here = os.path.dirname(os.path.abspath(__file__))
    env = jinja2.Environment(
        loader=jinja2.FileSystemLoader(os.path.join(here, "..", "templates")),
        trim_blocks=True,
        undefined=jinja2.StrictUndefined,
        keep_trailing_newline=True,
    )

    def lookup(kind, term, wantlist=False, default=None):
        if kind == "varnames":
            return sorted(k for k in context if re.search(term, k))
        if kind == "vars":
            return context.get(term, default)
        raise ValueError("unstubbed lookup: %s" % kind)

    def now(utc=False, fmt=None):
        assert utc and fmt == "%Y-%m-%d", "the row template must ask for a UTC date"
        return today

    env.filters["bool"] = to_bool
    env.filters["to_json"] = json.dumps
    env.filters["to_nice_json"] = lambda v: json.dumps(v, indent=4, sort_keys=True)
    env.globals["lookup"] = lookup
    env.globals["now"] = now

    rows = json.loads(env.get_template("isitup-rows.json.j2").render(**context))
    ctx = dict(context, isitup_rows=rows, isitup_rows_sha256=rows_sha256(rows),
               isitup_rows_source=SOURCE, isitup_rows_rendered_at=RENDERED_AT)
    services_env = env.get_template("services.env.j2").render(**ctx)
    meta = json.loads(env.get_template("meta.json.j2").render(**ctx))
    return rows, services_env, meta


def main():
    _, services_env, _ = render(CONTEXT)
    with open(sys.argv[1], "w", encoding="utf-8", newline="\n") as f:
        f.write(services_env)
    json.dump({"rows": EXPECTED}, sys.stdout)


if __name__ == "__main__":
    main()
