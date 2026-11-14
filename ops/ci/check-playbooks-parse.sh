#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# check-playbooks-parse.sh — run `ansible-playbook --syntax-check` on every
# playbook in ops/ansible and print the error of each one that fails.
#
# WHY THIS EXISTS: the loop used to live inline in ci.yml with its output sent
# to /dev/null, so a failure printed "syntax: <file>" and nothing else. A
# failure that did not reproduce on rerun left no record of its cause. This
# script keeps each playbook's output and prints it in full when the check
# fails, with the exit code and how long the check took.
#
# Usage: check-playbooks-parse.sh [--self-test] [playbook ...]
#   With no playbooks, checks every *.yml in ops/ansible except the files below.
#   ANSIBLE_DIR overrides the directory (default: ops/ansible in this repo).
set -euo pipefail

# Not playbooks: data files and a tasks file imported by relay-server.yml.
# site-bootstrap, site-bootstrap-for (imports it) and site-resume index a group
# named from site_id, which needs an inventory.
SKIP=" netbird_acl_matrix.yml netbird_acl_taxonomy.yml requirements.yml relay-server-rathole.yml site-bootstrap.yml site-bootstrap-for.yml site-resume.yml "

check() {
  # $1 = directory, then the playbooks to check (relative to it).
  local dir=$1 out p rc start end failed=0 n=0
  shift
  out="$(mktemp)"
  for p in "$@"; do
    n=$((n + 1))
    start=$(date +%s)
    rc=0
    (cd "$dir" && ansible-playbook --syntax-check -i localhost, \
      -e target=all -e mgmt_host_target=all -e migrator_host=all \
      -e rathole_client_target=all -e site_id=zagros "$p") >"$out" 2>&1 || rc=$?
    end=$(date +%s)
    if [ "$rc" -ne 0 ]; then
      failed=$((failed + 1))
      echo "syntax: $p (exit $rc after $((end - start))s)"
      sed 's/^/    /' "$out"
    fi
  done
  rm -f "$out"
  if [ "$failed" -ne 0 ]; then
    echo "check-playbooks-parse: FAIL: $failed of $n playbook(s) did not parse"
    return 1
  fi
  echo "check-playbooks-parse: OK: $n playbook(s) parse"
}

self_test() (
  t="$(mktemp -d)"
  trap 'rm -rf "$t"' EXIT
  printf -- '- hosts: all\n  tasks:\n    - ansible.builtin.debug:\n        msg: ok\n' >"$t/good.yml"
  printf -- '- hosts: all\n  tasks:\n    - ansible.builtin.debug:\n      msg: [unclosed\n' >"$t/bad.yml"
  if ! check "$t" good.yml >"$t/out" 2>&1; then
    echo "self-test: a valid playbook was refused" >&2
    cat "$t/out" >&2
    exit 1
  fi
  if check "$t" good.yml bad.yml >"$t/out" 2>&1; then
    echo "self-test: a broken playbook passed" >&2
    exit 1
  fi
  # The point of the script: the failure carries ansible's own error, not just
  # the file name.
  if ! grep -q '^syntax: bad.yml (exit [1-9]' "$t/out" || ! grep -q '^    .*ERROR' "$t/out"; then
    echo "self-test: the failure did not print ansible's error" >&2
    cat "$t/out" >&2
    exit 1
  fi
  echo "OK: self-test (a valid playbook passes; a broken one fails and prints ansible's error)"
)

if [ "${1:-}" = "--self-test" ]; then
  self_test
  exit $?
fi

dir="${ANSIBLE_DIR:-$(git rev-parse --show-toplevel)/ops/ansible}"
if [ "$#" -gt 0 ]; then
  check "$dir" "$@"
  exit $?
fi
set --
for f in "$dir"/*.yml; do
  b="$(basename "$f")"
  case "$SKIP" in *" $b "*) continue ;; esac
  set -- "$@" "$b"
done
check "$dir" "$@"
