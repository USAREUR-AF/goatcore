#!/usr/bin/env python3
"""check-no-secret-in-exec-argv: no secret on a `docker exec` command line in the checked roles.

A value on a command line is in the host's process list (ps, /proc/<pid>/cmdline) for as long
as the command runs, readable by any local user. A secret goes to the container another way:
psql over the container's local socket needs no password at all; otherwise stdin, or an
--env-file of mode 0600.

For every task in the checked roles' tasks/ and handlers/ files that runs `docker exec`
(ansible.builtin.command with argv or cmd, or ansible.builtin.shell), this refuses an argument
that:
  - is NAME=VALUE with a templated value and a NAME that reads as a secret
    (PGPASSWORD=..., *PASSWORD*, *SECRET*, *TOKEN*, *_KEY), or
  - templates a variable whose name reads as a secret ({{ ..._password }}, ..._secret,
    ..._token, ..._key), or
  - follows -e / --env and names a secret, or is --password=....
A secret in `stdin:` is allowed: stdin is not on the command line.

  check-no-secret-in-exec-argv.py [role ...]        default: ory
  check-no-secret-in-exec-argv.py --self-test
Exit 0 clean, 1 a secret on a command line, 2 a file could not be read.
"""
from __future__ import annotations

import re
import shlex
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
DEFAULT_ROLES = ["ory"]
SECRET_NAME = re.compile(r"(?i)(passw(or)?d|secret|token|(^|_)key$|_key\b|apikey)")
TEMPLATED_VAR = re.compile(r"\{\{\s*([A-Za-z_][A-Za-z0-9_]*)")
MODULES = ("ansible.builtin.command", "command", "ansible.builtin.shell", "shell")


def tasks_of(doc):
    """Every task dict, blocks flattened."""
    for t in doc or []:
        if not isinstance(t, dict):
            continue
        yield t
        for k in ("block", "rescue", "always"):
            yield from tasks_of(t.get(k))


def argv_of(task) -> list[str] | None:
    for m in MODULES:
        if m not in task:
            continue
        body = task[m]
        if isinstance(body, dict):
            if isinstance(body.get("argv"), list):
                return [str(a) for a in body["argv"]]
            cmd = body.get("cmd")
        else:
            cmd = body
        if isinstance(cmd, str):
            try:
                return shlex.split(cmd)
            except ValueError:
                return cmd.split()
    if isinstance(task.get("args"), dict) and isinstance(task["args"].get("argv"), list):
        return [str(a) for a in task["args"]["argv"]]
    return None


def secret_args(argv: list[str]) -> list[str]:
    if "docker" not in [Path(a).name for a in argv[:1]] and "docker" not in argv[:2]:
        return []
    if "exec" not in argv:
        return []
    found = []
    for i, a in enumerate(argv):
        name, eq, value = a.partition("=")
        if eq and "{{" in value and SECRET_NAME.search(name.lstrip("-")):
            found.append(a)
            continue
        for var in TEMPLATED_VAR.findall(a):
            if SECRET_NAME.search(var):
                found.append(a)
                break
        else:
            if i and argv[i - 1] in ("-e", "--env") and SECRET_NAME.search(name):
                found.append(a)
    return found


def loader():
    """yaml.SafeLoader that reads ansible's !unsafe and !vault tags as plain strings."""
    import yaml

    class L(yaml.SafeLoader):
        pass
    for tag in ("!unsafe", "!vault"):
        L.add_constructor(tag, lambda ld, node: ld.construct_scalar(node))
    return L


def check(repo: Path, roles: list[str]) -> tuple[list[str], int]:
    import yaml
    problems, seen = [], 0
    ld = loader()
    for role in roles:
        base = repo / "ops/ansible/roles" / role
        if not base.is_dir():
            raise OSError(f"no role {role} under ops/ansible/roles")
        for sub in ("tasks", "handlers"):
            for f in sorted((base / sub).glob("*.y*ml")) if (base / sub).is_dir() else []:
                doc = yaml.load(f.read_text(encoding="utf-8"), Loader=ld)
                for t in tasks_of(doc):
                    argv = argv_of(t)
                    if argv is None:
                        continue
                    seen += 1
                    for a in secret_args(argv):
                        problems.append(f"{f.relative_to(repo)}: task {t.get('name', '(unnamed)')!r} puts a secret on the "
                                        f"docker exec command line: {a.split('=', 1)[0]}=...")
    return problems, seen


def self_test() -> int:
    failures = []
    with tempfile.TemporaryDirectory() as tmp:
        repo = Path(tmp)
        tasks = repo / "ops/ansible/roles/r/tasks"
        tasks.mkdir(parents=True)

        def run(yml: str):
            (tasks / "main.yml").write_text(yml)
            return check(repo, ["r"])[0]

        cases = [
            ("PGPASSWORD on -e", "- name: a\n  ansible.builtin.command:\n    argv: [docker, exec, -e, \"PGPASSWORD={{ pg_pw }}\", c, psql]\n", 1),
            ("a templated *_password", "- name: a\n  ansible.builtin.command:\n    argv: [docker, exec, c, psql, -c, \"ALTER ROLE x PASSWORD '{{ x_password }}'\"]\n", 1),
            ("a cmd string", "- name: a\n  ansible.builtin.command:\n    cmd: docker exec -e API_TOKEN={{ t }} c true\n", 1),
            ("inside a block", "- name: b\n  block:\n    - name: a\n      ansible.builtin.shell: docker exec -e DB_SECRET={{ s }} c true\n", 1),
            ("on stdin", "- name: a\n  ansible.builtin.command:\n    argv: [docker, exec, -i, c, psql]\n    stdin: \"ALTER ROLE x PASSWORD '{{ x_password }}'\"\n", 0),
            ("no secret", "- name: a\n  ansible.builtin.command:\n    argv: [docker, exec, -u, postgres, c, psql, -U, postgres, -c, \"SELECT 1\"]\n", 0),
            ("an !unsafe argument", "- name: a\n  ansible.builtin.command:\n    argv: [docker, image, inspect, --format, !unsafe '{{json .Config}}', i]\n", 0),
            ("not docker exec", "- name: a\n  ansible.builtin.command:\n    argv: [curl, \"-H\", \"X={{ api_token }}\"]\n", 0),
        ]
        for what, yml, want in cases:
            got = len(run(yml))
            if got != want:
                failures.append(f"{what}: {got} finding(s), want {want}")
    for f in failures:
        print(f"  FAIL {f}")
    if failures:
        print(f"check-no-secret-in-exec-argv: self-test FAILED ({len(failures)})")
        return 1
    print(f"check-no-secret-in-exec-argv: self-test passed ({len(cases)} cases)")
    return 0


def main(argv: list[str]) -> int:
    try:
        import yaml  # noqa: F401
    except ImportError as e:
        print(f"check-no-secret-in-exec-argv: missing dependency: {e}", file=sys.stderr)
        return 2
    if argv[1:] == ["--self-test"]:
        return self_test()
    roles = argv[1:] or DEFAULT_ROLES
    try:
        problems, seen = check(REPO, roles)
    except (OSError, Exception) as e:  # noqa: BLE001 - an unreadable file is a refusal, not a pass
        print(f"check-no-secret-in-exec-argv: {e}", file=sys.stderr)
        return 2
    if seen == 0:
        print("check-no-secret-in-exec-argv: no command task read; refusing to report clean", file=sys.stderr)
        return 2
    for p in problems:
        print(p)
    if problems:
        return 1
    print(f"check-no-secret-in-exec-argv: PASS, {seen} command task(s) in {', '.join(roles)}: no secret on a docker exec command line")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
