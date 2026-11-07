#!/usr/bin/env python3
"""Join live P12 role/inline lookups and cookie consumers to generator SPEC.

YAML comments are not demand. Exact Jinja expressions are resolved through
play vars and role defaults; unsupported expressions fail instead of becoming
a false coverage pass. No operator files, environment values or stores read.
"""
import argparse
import json
from pathlib import Path
import re
import subprocess
import sys

import yaml


class Invalid(ValueError):
    pass


def read_yaml(path):
    return yaml.load(path.read_text(), Loader=yaml.BaseLoader)


def secret_env(expression, variables, seen=()):
    env = re.fullmatch(r"\{\{\s*lookup\('env',\s*'([A-Z][A-Z0-9_]*)'\)\s*\}\}", expression)
    if env:
        return env[1]
    var = re.fullmatch(r"\{\{\s*([a-z][a-z0-9_]*)\s*\}\}", expression)
    if not var or var[1] in seen or var[1] not in variables:
        raise Invalid("unsupported or missing secret binding in P12")
    return secret_env(variables[var[1]], variables, (*seen, var[1]))


def values_named(value, key):
    if isinstance(value, dict):
        for name, child in value.items():
            if name == key:
                yield child
            yield from values_named(child, key)
    elif isinstance(value, list):
        for child in value:
            yield from values_named(child, key)


def demanded(root, play, goatnet):
    snake = goatnet.replace("-", "_")
    variables = play["vars"]
    demanded_names = set()
    roles = variables["p12_role_client_secrets"]
    if not isinstance(roles, dict) or not roles:
        raise Invalid("P12 role client contract is empty or invalid")
    declared = [role["role"] if isinstance(role, dict) else role for role in play["roles"]]
    if len(set(declared)) != len(declared) or set(declared) != set(roles):
        raise Invalid("P12 role list and preflight map differ")
    for role, binding in roles.items():
        if not re.fullmatch(r"[a-z0-9-]+", role):
            raise Invalid("invalid P12 role name")
        defaults = read_yaml(root / "ops/ansible/roles" / role / "defaults/main.yml") or {}
        tasks = read_yaml(root / "ops/ansible/roles" / role / "tasks/main.yml")
        bindings = list(values_named(tasks, "client_secret"))
        actual = {secret_env(expr, {**defaults, **variables}) for expr in bindings}
        expected = secret_env(binding, variables)
        if actual != {expected}:
            raise Invalid(f"{role}: preflight and registration consume different secrets")
        demanded_names.add(expected)
    clients = variables[f"{snake}_extra_oidc_clients"]
    ids = [client["client_id"] for client in clients]
    if len(set(ids)) != len(ids) or set(ids) & set(roles):
        raise Invalid("duplicate P12 client identifiers")
    for client in clients:
        demanded_names.add(secret_env(client["client_secret"], variables))
    families = {name.removesuffix("_OIDC_CLIENT_SECRET").removesuffix("_CLIENT_SECRET")
                for name in demanded_names}
    # Paired cookies are actual group_vars lookups for the P12 clients,
    # including cloud-hosted fences. Other services are outside this contract.
    for group in (f"{snake}_hosts", f"{snake}_relay_health"):
        for path in (root / "ops/ansible/group_vars" / group).glob("*.yml"):
            data = read_yaml(path)
            for value in data.values():
                if isinstance(value, str):
                    match = re.fullmatch(r"\{\{\s*lookup\('env',\s*'(" + snake.upper() + r"_[A-Z0-9_]*COOKIE_SECRET)'\)\s*\}\}", value)
                    if match and match[1].removesuffix("_COOKIE_SECRET") in families:
                        demanded_names.add(match[1])
    return demanded_names


def check(root, play, spec, goatnet):
    demand = demanded(root, play, goatnet)
    missing = demand - set(spec)
    if missing:
        raise Invalid("generator omits: " + ", ".join(sorted(missing)))
    if any(spec[name] != "hex48" for name in demand if name.endswith("CLIENT_SECRET")):
        raise Invalid("OIDC client secrets must use hex48")
    if any(spec[name] != "hex32" for name in demand if name.endswith("COOKIE_SECRET")):
        raise Invalid("proxy cookies must use hex32")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[2])
    parser.add_argument("--goatnet", default="zagros-prod",
                        help="the goatnet whose <goatnet>-oidc-clients.yml play and <goatnet>_hosts group_vars are checked")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    root = args.root
    goatnet = args.goatnet
    if not re.fullmatch(r"[a-z0-9][a-z0-9-]*", goatnet):
        raise Invalid(f"invalid goatnet name {goatnet!r}")
    play_path = root / f"ops/ansible/{goatnet}-oidc-clients.yml"
    if not play_path.is_file():
        raise Invalid(f"{play_path.relative_to(root)} is not in this tree; the check runs in the goatnet's "
                      "environment repository against the assembled tree (--goatnet <name>)")
    play = read_yaml(play_path)[0]
    spec = json.loads(subprocess.check_output(["/bin/bash", str(root / "ops/scripts/newsite-secrets.sh"),
                                              goatnet, "--spec-json"], text=True))
    if args.self_test:
        # Exact membership, not substring matching. Removing an actual live
        # client's generator row must fail before trusting the positive verdict.
        missing = dict(spec)
        first = secret_env(next(iter(play["vars"]["p12_role_client_secrets"].values())), play["vars"])
        del missing[first]
        missing[first + "_EXTRA"] = "hex48"
        try:
            check(root, play, missing, goatnet)
        except Invalid:
            print("self-test: missing exact client family rejected")
        else:
            raise Invalid("self-test false pass")
    check(root, play, spec, goatnet)
    print(f"newsite credential coverage: {len(demanded(root, play, goatnet))} {goatnet} live demands covered")


if __name__ == "__main__":
    try:
        main()
    except (Invalid, KeyError, TypeError, OSError, subprocess.SubprocessError, ValueError) as error:
        print(f"FAIL: {error}", file=sys.stderr)
        sys.exit(1)
