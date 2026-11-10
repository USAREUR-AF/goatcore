#!/usr/bin/env python3
"""ops/ansible/tests/var-walker/walk.py — pre-deploy required-var check.

Parses every `ansible.builtin.assert:` task across every role under
ops/ansible/roles/, collects the var names referenced by `that:` clauses,
and cross-checks each against the declared sources available at invocation
time (group_vars + inventory host_vars + bootstrap `-e` args).

Reports any required var that is NOT provided, BEFORE the playbook runs.
Would have caught 5 of the 9 live iterations of one site bootstrap on
2026-04-20.

Usage:
    python3 ops/ansible/tests/var-walker/walk.py \\
        --roles ops/ansible/roles \\
        --bootstrap ops/ansible/<site>-bootstrap.yml \\
        --inventory ops/ansible/inventory/mgmt-hosts.ini \\
        --group-vars ops/ansible/group_vars/<site>_hosts/vars.yml \\
        --bootstrap-e-args 'postgres_superuser_password,netbird_encryption_key,pgbackrest_cipher_pass' \\
        --group <site>_hosts

Exit 0 = every required var is provided somewhere; exit 1 = gap.
"""
from __future__ import annotations

import argparse
import re
import sys
import pathlib
from typing import Iterable

try:
    import yaml
except ImportError:
    sys.stderr.write("pip install pyyaml\n")
    sys.exit(2)


ASSERT_VAR_RE = re.compile(
    r"""(?x)
    (?P<name>[a-zA-Z_][a-zA-Z0-9_]*)     # var name
    \s+is\s+defined                       # literal
    (?:\s+and\s+(?P=name)\s*\|\s*length   # optional '&& length > N' clause
       \s*>[=]?\s*\d+)?
    """
)


def walk_roles(roles_dir: pathlib.Path) -> dict[str, dict[str, set[str]]]:
    """Return map of role -> {'hard': set[str], 'soft': set[str]}.

    'hard' = always-required (assert has no `when:` clause).
    'soft' = conditionally-required (assert has a `when:` that may guard
             it out at runtime, e.g. `when: pgbackrest_s3_enabled`).
    """
    req: dict[str, dict[str, set[str]]] = {}
    for tasks_main in roles_dir.glob("*/tasks/main.yml"):
        role = tasks_main.parent.parent.name
        req.setdefault(role, {"hard": set(), "soft": set()})
        try:
            with tasks_main.open() as f:
                tasks = yaml.safe_load(f) or []
        except yaml.YAMLError as e:
            print(f"WARN: {tasks_main}: yaml parse: {e}", file=sys.stderr)
            continue
        for task in tasks:
            if not isinstance(task, dict):
                continue
            a = task.get("ansible.builtin.assert") or task.get("assert")
            if not a:
                continue
            conditional = "when" in task
            that = a.get("that")
            if isinstance(that, str):
                that = [that]
            for clause in that or []:
                m = ASSERT_VAR_RE.search(clause)
                if m:
                    bucket = "soft" if conditional else "hard"
                    req[role][bucket].add(m.group("name"))
    return req


def vars_from_file(path: pathlib.Path) -> set[str]:
    """Top-level keys of a YAML vars file."""
    if not path.exists():
        return set()
    with path.open() as f:
        data = yaml.safe_load(f) or {}
    return set(data.keys()) if isinstance(data, dict) else set()


def vars_from_inventory_group(inv_path: pathlib.Path, group: str) -> set[str]:
    """Parse an INI inventory and return every var name declared on any host
    in the requested group OR any group:vars section for that group."""
    if not inv_path.exists():
        return set()
    found: set[str] = set()
    current_section = ""
    in_target_group = False
    with inv_path.open() as f:
        for line in f:
            line = line.strip()
            if not line or line.startswith("#") or line.startswith(";"):
                continue
            if line.startswith("[") and line.endswith("]"):
                current_section = line[1:-1]
                # kwt_hosts, or kwt_hosts:vars, or kwt_hosts:children
                base = current_section.split(":", 1)[0]
                in_target_group = (base == group) and (":children" not in current_section)
                continue
            if in_target_group and "=" in line:
                # per-host vars: "hostname key=val key=val"
                for tok in line.split()[1:]:
                    if "=" in tok:
                        k, _, _ = tok.partition("=")
                        found.add(k)
            elif current_section == f"{group}:vars" and "=" in line:
                k, _, _ = line.partition("=")
                found.add(k.strip())
    return found


def bootstrap_var_overrides(bootstrap_yml: pathlib.Path) -> set[str]:
    """Keys set in the bootstrap's import_playbook `vars:` or per-play `vars:`.
    These override what the imported playbook's role-level expectations are."""
    if not bootstrap_yml.exists():
        return set()
    with bootstrap_yml.open() as f:
        plays = yaml.safe_load(f) or []
    found: set[str] = set()
    for play in plays or []:
        if not isinstance(play, dict):
            continue
        v = play.get("vars") or {}
        if isinstance(v, dict):
            found.update(v.keys())
    return found


def ansible_magic_vars() -> set[str]:
    """Vars Ansible provides automatically — not a gap if referenced."""
    return {
        "ansible_host", "ansible_user", "ansible_connection",
        "ansible_python_interpreter", "ansible_aws_ssm_region",
        "inventory_hostname", "groups", "hostvars",
    }


def main(argv: Iterable[str] | None = None) -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--roles", required=True, type=pathlib.Path)
    p.add_argument("--bootstrap", required=True, type=pathlib.Path)
    p.add_argument("--inventory", required=True, type=pathlib.Path)
    p.add_argument("--group-vars", required=True, type=pathlib.Path,
                   help="group_vars YAML for the target group")
    p.add_argument("--bootstrap-e-args", default="",
                   help="comma-separated list of vars you pass via -e")
    p.add_argument("--group", required=True,
                   help="inventory group to check (e.g. kwt_hosts)")
    args = p.parse_args(list(argv) if argv is not None else None)

    required_by_role = walk_roles(args.roles)

    provided = set()
    provided |= vars_from_file(args.group_vars)
    provided |= vars_from_inventory_group(args.inventory, args.group)
    provided |= {v.strip() for v in args.bootstrap_e_args.split(",") if v.strip()}
    provided |= bootstrap_var_overrides(args.bootstrap)
    provided |= ansible_magic_vars()

    print("=== Required-var walker ===")
    print(f"group: {args.group}")
    print(f"roles scanned: {', '.join(sorted(required_by_role))}")
    print(f"providers scanned:")
    print(f"  group_vars: {args.group_vars}")
    print(f"  inventory: {args.inventory} [{args.group}]")
    print(f"  bootstrap imports: {args.bootstrap}")
    print(f"  -e args passed in: {args.bootstrap_e_args or '(none)'}")
    print()

    failures: list[tuple[str, str]] = []
    warnings: list[tuple[str, str]] = []
    for role, buckets in sorted(required_by_role.items()):
        hard = buckets["hard"]
        soft = buckets["soft"]
        hard_missing = hard - provided
        soft_missing = soft - provided
        if hard_missing:
            label = "GAP  "
        elif soft_missing:
            label = "WARN "
        else:
            label = "OK   "
        all_needs = sorted(hard | soft) or "(none)"
        print(f"[{label}] {role} — requires {all_needs}")
        if hard_missing:
            print(f"         HARD-MISSING: {sorted(hard_missing)}  (unconditional assert, will fail at runtime)")
            for var in sorted(hard_missing):
                failures.append((role, var))
        if soft_missing:
            print(f"         SOFT-MISSING: {sorted(soft_missing)}  (gated by a when: clause — check the flag is disabling them as intended)")
            for var in sorted(soft_missing):
                warnings.append((role, var))

    print()
    if failures:
        print(f"FAIL — {len(failures)} HARD-required var(s) not provided:")
        for role, var in failures:
            print(f"  {role}: {var}")
        return 1
    if warnings:
        print(f"PASS (with {len(warnings)} soft-missing var(s) — verify the guarding when: flag is disabling them):")
        for role, var in warnings:
            print(f"  {role}: {var}")
        return 0
    print("PASS — every role-required var is provided by some source.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
