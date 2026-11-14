#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# check-compose-memory-is-variable.sh — every container memory limit and
# reservation in a compose file or role compose template is a variable with a
# default, never a literal.
#
# WHY THIS EXISTS: a limit written as a literal cannot be set for the host it
# runs on. A 4 GB node got Postgres at 2G and the fabric crawler at 3,968M with
# no way to lower either (docs/design/phone-node.md). Each limit is now a
# variable whose default is the value it had: in a role template a
# `{{ var }}` with the default in the role's defaults/main.yml, in a plain
# compose file `${KEY:-default}`, which the playbook that renders that
# directory's .env sets from a group_var.
#
# WHAT IT CHECKS:
#   1. In every compose file under ops/ (docker-compose*.yml, compose*.yml)
#      and every role template whose name contains "compose", a `memory:`,
#      `mem_limit:`, `mem_reservation:` or `memswap_limit:` value must be a
#      Jinja expression or `${KEY:-default}`. A bare `${KEY}` is refused: it
#      has no default. Test fixtures (tests/, testdata/) are not deployed and
#      are skipped.
#   2. Every such KEY in ops/compose/mgmt-host/ is written by both of its .env
#      writers, ops/ansible/mgmt-host.yml and
#      ops/ansible/tasks/mgmt-host-ceilings-env.yml, and every KEY in
#      ops/compose/goat-obs/ by ops/ansible/obs.yml. A key neither writes can be
#      set by no group_var.
#
# Usage: check-compose-memory-is-variable.sh [--self-test]
#   CHECK_ROOT overrides the tree scanned (default: the repository root).
set -euo pipefail

scan() {
  python3 - "$1" <<'PY'
import os, re, sys

root = sys.argv[1]
KEYS = r'(?:memory|mem_limit|mem_reservation|memswap_limit)'
line_re = re.compile(r'^\s*' + KEYS + r'\s*:\s*(\S.*?)\s*$')
env_re = re.compile(r'\$\{([A-Z0-9_]+)(:-[^}]+)?\}')

def compose_files():
    for d, dirs, files in os.walk(os.path.join(root, 'ops')):
        rel = os.path.relpath(d, root)
        parts = rel.split(os.sep)
        if 'tests' in parts or 'testdata' in parts or 'node_modules' in parts:
            dirs[:] = []
            continue
        for f in files:
            in_templates = 'templates' in parts and 'roles' in parts
            if re.match(r'(docker-)?compose[^/]*\.ya?ml$', f) or (in_templates and 'compose' in f and f.endswith('.j2')):
                yield os.path.join(rel, f)

bad, keys_by_dir, checked = [], {}, 0
for rel in sorted(compose_files()):
    for n, line in enumerate(open(os.path.join(root, rel), encoding='utf-8'), 1):
        if line.lstrip().startswith('#'):
            continue
        m = line_re.match(line)
        if not m:
            continue
        value = re.sub(r'\s+#.*$', '', m.group(1)).strip('"\'')
        checked += 1
        if '{{' in value:
            continue
        e = env_re.search(value)
        if e and e.group(2):
            keys_by_dir.setdefault(os.path.dirname(rel), set()).add(e.group(1))
            continue
        why = 'has no default' if e else 'is a literal'
        bad.append(f'{rel}:{n}: {line.strip()}  ({why})')

writers = {
    'ops/compose/mgmt-host': ['ops/ansible/mgmt-host.yml', 'ops/ansible/tasks/mgmt-host-ceilings-env.yml'],
    'ops/compose/goat-obs': ['ops/ansible/obs.yml'],
}
for d, files in writers.items():
    for w in files:
        p = os.path.join(root, w)
        if not os.path.exists(p):
            continue
        text = open(p, encoding='utf-8').read()
        for k in sorted(keys_by_dir.get(d, ())):
            if not re.search(r'\b' + k + r'\b', text):
                bad.append(f'{w}: does not write {k}, which {d}/ reads as a memory setting')

if bad:
    print(f'check-compose-memory-is-variable: FAIL — {len(bad)} memory setting(s) cannot be set per host')
    for b in bad:
        print(f'    {b}')
    print('  Make it a variable whose default is the present value: `{{ <role>_memory_limit }}`')
    print('  with the default in the role\'s defaults/main.yml, or `${KEY:-<value>}` in a plain')
    print('  compose file, written to .env from a group_var by the playbook that renders it.')
    sys.exit(1)
print(f'check-compose-memory-is-variable: OK — {checked} memory setting(s), all variables with defaults')
PY
}

self_test() (
  t="$(mktemp -d)"
  trap 'rm -rf "$t"' EXIT
  mkdir -p "$t/ops/ansible/roles/r/templates" "$t/ops/compose/x" "$t/ops/ansible/roles/r/tests"
  cat >"$t/ops/ansible/roles/r/templates/docker-compose.yml.j2" <<'EOF'
services:
  a:
    deploy:
      resources:
        limits:
          memory: "{{ r_memory_limit }}"
EOF
  cat >"$t/ops/compose/x/docker-compose.yml" <<'EOF'
services:
  b:
    deploy:
      resources:
        limits:
          memory: "${B_MEMORY:-256M}"
EOF
  # A fixture under tests/ is not deployed: never flagged.
  printf 'services:\n  c:\n    mem_limit: 64M\n' >"$t/ops/ansible/roles/r/tests/docker-compose.yml"
  rc=0
  scan "$t" >/dev/null || rc=$?
  if [ "$rc" -ne 0 ]; then
    echo "self-test: variables were refused" >&2
    return 1
  fi
  printf 'services:\n  d:\n    mem_limit: 512m\n' >"$t/ops/compose/x/compose.extra.yml"
  rc=0
  scan "$t" >/dev/null || rc=$?
  if [ "$rc" -ne 1 ]; then
    echo "self-test: a literal mem_limit passed" >&2
    return 1
  fi
  rm "$t/ops/compose/x/compose.extra.yml"
  sed 's/memory: .*/memory: 1G/' "$t/ops/ansible/roles/r/templates/docker-compose.yml.j2" >"$t/tmp" && mv "$t/tmp" "$t/ops/ansible/roles/r/templates/docker-compose.yml.j2"
  rc=0
  scan "$t" >/dev/null || rc=$?
  if [ "$rc" -ne 1 ]; then
    echo "self-test: a literal memory limit in a role template passed" >&2
    return 1
  fi
  sed 's/memory: .*/memory: "{{ r_memory_limit }}"/' "$t/ops/ansible/roles/r/templates/docker-compose.yml.j2" >"$t/tmp" && mv "$t/tmp" "$t/ops/ansible/roles/r/templates/docker-compose.yml.j2"
  # shellcheck disable=SC2016  # a literal ${B_MEMORY} is the fixture
  sed 's/memory: .*/memory: "${B_MEMORY}"/' "$t/ops/compose/x/docker-compose.yml" >"$t/tmp" && mv "$t/tmp" "$t/ops/compose/x/docker-compose.yml"
  rc=0
  scan "$t" >/dev/null || rc=$?
  if [ "$rc" -ne 1 ]; then
    echo "self-test: \${KEY} with no default passed" >&2
    return 1
  fi
  echo "OK: self-test (variables pass; a literal mem_limit, a literal memory limit and \${KEY} with no default fail)"
)

if [ "${1:-}" = "--self-test" ]; then
  self_test
  exit $?
fi
scan "${CHECK_ROOT:-$(git rev-parse --show-toplevel)}"
