#!/usr/bin/env python3
"""Validate a goatbuilder config against the versioned schema, offline.

This is the `goatbuilder validate` verb before the binary exists: the first
conformance test every repository runs, needing no environment. It checks the
shape against ops/goatbuilder/schema/goatbuilder-v1.schema.json and then the
cross-field rules JSON Schema cannot express:

  - environment names are unique;
  - every stage of kind rung names a declared environment;
  - a stage that names an environment names a declared one;
  - every class override sits under a declared environment (by construction),
    and a pull-agent class under a push environment is allowed — that is the
    per-class adoption the design calls for;
  - under preset lite (ADR 1213): environments may be absent only when a lite:
    block stands in for them, and a lite: block beside explicit environments
    must name one of them. The rules after that read the derived shape, the
    same one goatbuilder's config.rs derives (derive_lite).

Usage:
    ./validate.py <config.yaml> [<config.yaml> ...]

Exit codes:
    0 — every config validates
    1 — at least one config failed
    2 — usage / file-not-found / dependency-not-installed error
"""
from __future__ import annotations

import copy
import json
import os
import sys

try:
    import yaml
except ImportError:
    sys.stderr.write("ERROR: PyYAML is required: pip install pyyaml\n")
    sys.exit(2)
try:
    from jsonschema import Draft7Validator
except ImportError:
    sys.stderr.write("ERROR: jsonschema is required: pip install jsonschema\n")
    sys.exit(2)

SCHEMA_PATH = os.path.join(os.path.dirname(os.path.abspath(__file__)), "schema", "goatbuilder-v1.schema.json")
CHECKOUT_CONFIG = os.path.join(os.path.dirname(os.path.abspath(__file__)), "goatbuilder.yaml")


def load_schema() -> dict:
    with open(SCHEMA_PATH) as fh:
        return json.load(fh)


def lite_problems(config: dict) -> list[str]:
    """The preset-lite rules (ADR 1213), on the config as written: under lite,
    environments may be absent only when a lite: block stands in for them; repo
    and components are taken from the checkout's config when it has one and may be
    absent when it does not; and a
    lite: block beside explicit environments must name one of them (its derived
    environment <name with - as _>, or a site <name>)."""
    if config.get("preset") != "lite":
        return []
    lite = config.get("lite")
    envs = config.get("environments")
    if envs is None and lite is None:
        return ["(root): preset lite: declare environments or a lite: block"]
    if envs is not None and lite is not None:
        name = lite["name"]
        name_us = name.replace("-", "_")
        for env in envs:
            if env.get("name") == name_us or any(s.get("name") == name for s in env.get("sites") or []):
                return []
        return [f"lite: lite.name '{name}' names no declared environment"]
    return []


def derive_lite(config: dict, checkout: dict | None = None) -> dict:
    """The preset's derivation (goatlite.md §3, §4): fill what a lite config leaves
    out, never overwriting an explicit field. repo and components come from the
    checkout's own config (`checkout`, ops/goatbuilder/goatbuilder.yaml: the
    repository the learner cloned declares them); the rest from the lite: block.
    The same defaults goatbuilder's config.rs derives; a copy of `config`, the
    input untouched."""
    if config.get("preset") != "lite":
        return config
    out = dict(config)
    if checkout is not None:
        for key in ("repo", "components"):
            if key not in out and key in checkout:
                out[key] = copy.deepcopy(checkout[key])
    if not isinstance(config.get("lite"), dict):
        return out
    name = config["lite"]["name"]
    name_us = name.replace("-", "_")
    if "environments" not in out:
        out["environments"] = [{
            "name": name_us,
            "converger": "push-ansible",
            "transport": "direct",
            "groups": [f"{name_us}_hosts"],
            "sites": [{"name": name, "role": "founder", "classes": {"core": {}}}],
        }]
    if "promotion" not in out:
        # The one rung is the goatnet itself: the environment the lite: block
        # names (its derived one, or the explicit one carrying site <name>).
        env_name = next((e["name"] for e in out["environments"]
                         if e.get("name") == name_us or any(s.get("name") == name for s in e.get("sites") or [])), name_us)
        out["promotion"] = {"path": [{"name": env_name, "kind": "rung", "environment": env_name}]}
    if "secrets" not in out:
        out["secrets"] = {"store": "env-file"}
    return out


# Words `goatbuilder federate <word>` takes, so none can name a partner. The one
# list is RESERVED_PEERING_NAMES in crates/goatbuilder/src/config.rs; a test there
# holds this copy equal to it.
RESERVED_PEERING_NAMES = ("init", "lint", "compose", "diff", "verify", "apply", "help",
                          "status", "plan", "deploy", "test", "allocate", "backout", "rotate",
                          "import")


def errors_for(config: dict, schema: dict | None = None, checkout: dict | None = None) -> list[str]:
    """Every problem with `config`, as 'where: what' lines; empty means valid.
    `checkout` is the checkout's own ops/goatbuilder/goatbuilder.yaml, when a lite
    config may take repo and components from it."""
    schema = schema or load_schema()
    out: list[str] = []
    for e in sorted(Draft7Validator(schema).iter_errors(config), key=lambda e: list(e.absolute_path)):
        where = "/".join(str(p) for p in e.absolute_path) or "(root)"
        out.append(f"{where}: {e.message}")
    if out:
        return out  # cross-field rules assume the shape is right
    out.extend(lite_problems(config))
    if out:
        return out
    config = derive_lite(config, checkout)  # the remaining rules read the derived shape
    envs = config.get("environments", [])
    names = [env["name"] for env in envs]
    dupes = sorted({n for n in names if names.count(n) > 1})
    if dupes:
        out.append(f"environments: duplicate name(s) {dupes}")
    declared = set(names)
    # sites: unique within an environment; a joiner joins a founder site in the same environment
    sites_of: dict[str, dict] = {}
    for ei, env in enumerate(envs):
        sites = env.get("sites") or []
        site_names = [s["name"] for s in sites]
        sdupes = sorted({n for n in site_names if site_names.count(n) > 1})
        if sdupes:
            out.append(f"environments/{ei}/sites: duplicate site name(s) {sdupes}")
        sites_of[env["name"]] = {s["name"]: s for s in sites}
        for si, s in enumerate(sites):
            if s.get("role") == "joiner":
                target = sites_of[env["name"]].get(s.get("joins"))
                if target is None or target is s or target.get("role", "founder") != "founder":
                    out.append(
                        f"environments/{ei}/sites/{si}: joiner '{s['name']}' must join a founder site "
                        f"in '{env['name']}', not '{s.get('joins')}'"
                    )
    for i, stage in enumerate(config.get("promotion", {}).get("path", [])):
        env = stage.get("environment")
        if env is not None and env not in declared:
            out.append(f"promotion/path/{i}: stage '{stage.get('name')}' names undeclared environment '{env}'")
    # peerings: the environment, the site and the edge class it names must all be declared
    for pi, p in enumerate(config.get("peerings") or []):
        env, site, cls = p.get("environment"), p.get("site"), p.get("edge_class")
        if env not in declared:
            out.append(f"peerings/{pi}: names undeclared environment '{env}'")
            continue
        s = sites_of.get(env, {}).get(site)
        if s is None:
            out.append(f"peerings/{pi}: names undeclared site '{site}' in '{env}'")
            continue
        if cls not in (s.get("classes") or {}):
            out.append(f"peerings/{pi}: edge_class '{cls}' is not a declared class at site '{site}'")
    # A word of `goatbuilder federate` never names a partner (GB-120). The binary also
    # refuses pins that collide; that needs its allocator (crates/goatbuilder/src/federate/alloc.rs).
    for pi, p in enumerate(config.get("peerings") or []):
        if p.get("with") in RESERVED_PEERING_NAMES:
            out.append(f"peerings/{pi} (with '{p.get('with')}'): '{p.get('with')}' is a word of `goatbuilder federate` "
                       f"and cannot name a partner (GB-120)")
    return out


def main() -> None:
    paths = sys.argv[1:]
    if not paths:
        sys.stderr.write(__doc__)
        sys.exit(2)
    schema = load_schema()
    # The checkout's own config, beside this script: what a lite config takes
    # repo and components from (the lite file lives at the checkout root).
    checkout = None
    if os.path.isfile(CHECKOUT_CONFIG):
        with open(CHECKOUT_CONFIG) as fh:
            checkout = yaml.safe_load(fh)
    failed = 0
    for path in paths:
        try:
            with open(path) as fh:
                config = yaml.safe_load(fh)
        except FileNotFoundError:
            sys.stderr.write(f"ERROR: {path}: not found\n")
            sys.exit(2)
        own = os.path.realpath(path) == os.path.realpath(CHECKOUT_CONFIG)
        problems = errors_for(config if isinstance(config, dict) else {}, schema, None if own else checkout)
        if problems:
            failed += 1
            print(f"FAIL {path}")
            for p in problems:
                print(f"     {p}")
        else:
            print(f"ok   {path}")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
